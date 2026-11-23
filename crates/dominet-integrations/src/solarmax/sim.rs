// SPDX-License-Identifier: AGPL-3.0-or-later
//! **Simulated** SolarMax inverter (tests and demonstrations): a local TCP server that speaks the protocol.
//! Nothing here addresses a real inverter: the simulator listens on `127.0.0.1`.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::protocol::{QUERY_TYPES, build_answer, checksum};

/// Behaviour of the simulator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Behaviour {
    /// Answers normally.
    Normal,
    /// Accepts connections but never answers.
    Silent,
    /// Answers with noise.
    Garbage,
}

/// Served values, in raw hexadecimal (as on the wire).
#[derive(Debug, Clone)]
pub struct SimState {
    pub inverter: u8,
    pub values: BTreeMap<String, String>,
    pub behaviour: Behaviour,
    /// Number of accepted connections.
    pub connections: u32,
}

impl Default for SimState {
    fn default() -> Self {
        let values = [
            ("UDC", "0C0A"), // 308.2 V
            ("IDC", "0130"), // 3.04 A
            ("UL1", "08FC"), // 230.0 V
            ("IL1", "0113"), // 2.75 A
            ("PAC", "0640"),
            ("KDY", "00C8"), // 20.0 kWh
            ("KT0", "1F40"),
            ("TKK", "002D"), // 45 degrees
            ("ADR", "1"),
            ("TYP", "4E3E"),   // 20030 : SolarMax 4200S
            ("PIN", "0FA0"),   // 2000 (4000 W)
            ("SYS", "4E24,0"), // 20004: operation on MPP
            ("SAL", "0"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        Self {
            inverter: 1,
            values,
            behaviour: Behaviour::Normal,
            connections: 0,
        }
    }
}

/// Running simulator; stops when dropped.
#[derive(Debug)]
pub struct SimInverter {
    pub addr: SocketAddr,
    pub state: Arc<Mutex<SimState>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for SimInverter {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl SimInverter {
    /// Listens on a free port of `127.0.0.1`.
    ///
    /// # Errors
    /// Error opening the port.
    pub async fn start(state: SimState) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let state = Arc::new(Mutex::new(state));
        let shared = Arc::clone(&state);
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                if let Ok(mut s) = shared.lock() {
                    s.connections += 1;
                }
                tokio::spawn(serve(stream, Arc::clone(&shared)));
            }
        });
        Ok(Self { addr, state, task })
    }

    /// Changes a served value (raw hexadecimal).
    pub fn set(&self, key: &str, raw: &str) {
        if let Ok(mut s) = self.state.lock() {
            s.values.insert(key.to_string(), raw.to_string());
        }
    }

    pub fn set_behaviour(&self, behaviour: Behaviour) {
        if let Ok(mut s) = self.state.lock() {
            s.behaviour = behaviour;
        }
    }

    #[must_use]
    pub fn connections(&self) -> u32 {
        self.state.lock().map_or(0, |s| s.connections)
    }
}

/// Reads the requested keys in `{FB;NN;LL|64:K1;K2|CCCC}` (checksum verified).
fn requested_keys(request: &str) -> Option<(u8, Vec<String>)> {
    let inner = request.strip_prefix('{')?.strip_suffix('}')?;
    let (content, sum) = inner.split_at(inner.len().checked_sub(4)?);
    if sum != checksum(content) {
        return None;
    }
    let mut parts = content.split('|');
    let header = parts.next()?;
    let body = parts.next()?.strip_prefix("64:")?;
    let mut fields = header.split(';');
    if fields.next()? != "FB" {
        return None;
    }
    let inverter = fields.next()?.parse().ok()?;
    let keys: Vec<String> = body.split(';').map(str::to_string).collect();
    keys.iter()
        .all(|k| QUERY_TYPES.contains(&k.as_str()))
        .then_some((inverter, keys))
}

async fn serve(mut stream: TcpStream, state: Arc<Mutex<SimState>>) {
    let mut request = String::new();
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte).await {
            Ok(1) => {}
            _ => return,
        }
        request.push(char::from(byte[0]));
        if byte[0] != b'}' {
            continue;
        }
        let snapshot = match state.lock() {
            Ok(s) => s.clone(),
            Err(_) => return,
        };
        let reply = match snapshot.behaviour {
            Behaviour::Silent => None,
            Behaviour::Garbage => Some("%%% bruit %%%}".to_string()),
            Behaviour::Normal => requested_keys(&request).and_then(|(inverter, keys)| {
                if inverter != snapshot.inverter {
                    return None;
                }
                let items: Option<Vec<(&str, &str)>> = keys
                    .iter()
                    .map(|k| snapshot.values.get(k).map(|v| (k.as_str(), v.as_str())))
                    .collect();
                items.map(|i| build_answer(inverter, &i))
            }),
        };
        request.clear();
        if let Some(reply) = reply
            && stream.write_all(reply.as_bytes()).await.is_err()
        {
            return;
        }
    }
}
