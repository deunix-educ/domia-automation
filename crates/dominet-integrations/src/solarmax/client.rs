// SPDX-License-Identifier: AGPL-3.0-or-later
//! Client TCP asynchrone d'un onduleur SolarMax : une connexion, des echanges requete / reponse bornes dans le temps.
//!
//! Comme l'implementation de reference, la connexion est abandonnee a la moindre erreur et reprise au cycle suivant. L'adresse de l'onduleur
//! n'est JAMAIS une valeur par defaut : elle vient de la configuration (`SOLARMAX_HOST`), pour qu'aucun essai ne contacte
//! l'onduleur de la production par megarde.

use std::collections::BTreeMap;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use super::protocol::{
    ALARM_CODES, INVERTER_TYPES, Production, ProtocolError, Value, build_query, parse_answer,
    status_text,
};

/// Grandeurs demandees a chaque cycle (comme `QUERY_MASK` de la reference).
pub const QUERY_MASK: [&str; 8] = ["UDC", "IDC", "UL1", "IL1", "PAC", "KDY", "KT0", "TKK"];
/// Taille maximale d'une reponse lue (une reponse normale fait une centaine d'octets).
const MAX_ANSWER: usize = 4096;

#[derive(Debug, thiserror::Error)]
pub enum SolarMaxError {
    #[error("connexion impossible : {0}")]
    Connect(String),
    #[error("echange impossible : {0}")]
    Io(String),
    #[error("l'onduleur n'a pas repondu a temps")]
    Timeout,
    #[error("pas de reponse de l'onduleur #{0}")]
    NoAnswer(u8),
    #[error("reponse illisible : {0}")]
    Protocol(#[from] ProtocolError),
    #[error("onduleur #{0} non detecte ou de type inconnu")]
    NotDetected(u8),
    #[error("onduleur #{inverter} : {status} ({errors})")]
    Alarm {
        inverter: u8,
        status: String,
        errors: String,
    },
}

/// Connexion a un onduleur.
#[derive(Debug)]
pub struct SolarMaxClient {
    host: String,
    port: u16,
    limit: Duration,
    stream: Option<TcpStream>,
    /// Puissance installee (`PIN`) de l'onduleur detecte sur la connexion courante.
    installed: Option<(u8, f64)>,
}

impl SolarMaxClient {
    #[must_use]
    pub fn new(host: &str, port: u16, limit: Duration) -> Self {
        Self {
            host: host.to_string(),
            port,
            limit,
            stream: None,
            installed: None,
        }
    }

    /// Oublie la connexion : le prochain cycle en ouvre une nouvelle.
    pub fn reset(&mut self) {
        self.stream = None;
        self.installed = None;
    }

    async fn stream(&mut self) -> Result<&mut TcpStream, SolarMaxError> {
        if self.stream.is_none() {
            let target = (self.host.as_str(), self.port);
            let connected = timeout(self.limit, TcpStream::connect(target))
                .await
                .map_err(|_| SolarMaxError::Timeout)?
                .map_err(|e| SolarMaxError::Connect(e.to_string()))?;
            self.stream = Some(connected);
        }
        self.stream
            .as_mut()
            .ok_or_else(|| SolarMaxError::Connect("connection lost".into()))
    }

    /// Envoie une requete et lit la reponse (jusqu'a `}`, les octets sont masques sur 7 bits comme en reference).
    async fn exchange(&mut self, request: &str) -> Result<String, SolarMaxError> {
        let limit = self.limit;
        let stream = self.stream().await?;
        let io = async {
            stream.write_all(request.as_bytes()).await?;
            let mut answer = String::new();
            let mut byte = [0u8; 1];
            loop {
                if stream.read(&mut byte).await? == 0 {
                    break;
                }
                let c = byte[0] & 0x7F;
                if c == 0 {
                    break;
                }
                answer.push(char::from(c));
                if c == b'}' || answer.len() >= MAX_ANSWER {
                    break;
                }
            }
            Ok::<_, std::io::Error>(answer)
        };
        timeout(limit, io)
            .await
            .map_err(|_| SolarMaxError::Timeout)?
            .map_err(|e| SolarMaxError::Io(e.to_string()))
    }

    /// Interroge l'onduleur `inverter`.
    ///
    /// # Errors
    /// Reseau, delai depasse, reponse absente ou illisible.
    pub async fn query(
        &mut self,
        inverter: u8,
        keys: &[&str],
    ) -> Result<BTreeMap<String, Value>, SolarMaxError> {
        let answer = self.exchange(&build_query(inverter, keys)?).await?;
        if answer.is_empty() {
            return Err(SolarMaxError::NoAnswer(inverter));
        }
        Ok(parse_answer(&answer)?.1)
    }

    /// Detecte l'onduleur (type connu, puissance installee) une fois par connexion.
    async fn installed(&mut self, inverter: u8) -> Result<f64, SolarMaxError> {
        if let Some((id, installed)) = self.installed
            && id == inverter
        {
            return Ok(installed);
        }
        let data = self.query(inverter, &["ADR", "TYP", "PIN"]).await?;
        let known = data
            .get("TYP")
            .and_then(Value::as_i64)
            .is_some_and(|t| INVERTER_TYPES.iter().any(|(code, _, _)| *code == t));
        let installed = data.get("PIN").and_then(Value::as_f64);
        match (known, installed) {
            (true, Some(pin)) => {
                self.installed = Some((inverter, pin));
                Ok(pin)
            }
            _ => Err(SolarMaxError::NotDetected(inverter)),
        }
    }

    /// Etat et alarmes (`SYS`, `SAL`).
    async fn status(&mut self, inverter: u8) -> Result<(String, String), SolarMaxError> {
        let data = self.query(inverter, &["SYS", "SAL"]).await?;
        let alarms = data.get("SAL").and_then(Value::as_i64).unwrap_or(0);
        let errors: Vec<&str> = ALARM_CODES
            .iter()
            .filter(|(bit, _)| alarms & bit != 0)
            .map(|(_, text)| *text)
            .collect();
        let status = match data.get("SYS") {
            Some(Value::Pair(state, _)) => status_text(*state),
            _ => "Unknown",
        };
        Ok((status.to_string(), errors.join(", ")))
    }

    /// Un cycle complet : detection, grandeurs, etat. A la moindre erreur la connexion est abandonnee.
    ///
    /// # Errors
    /// [`SolarMaxError`] ; une alarme de l'onduleur est aussi une erreur (rien n'est publie, comme en reference).
    pub async fn read_production(&mut self, inverter: u8) -> Result<Production, SolarMaxError> {
        let result = self.cycle(inverter).await;
        if result.is_err() {
            self.reset();
        }
        result
    }

    async fn cycle(&mut self, inverter: u8) -> Result<Production, SolarMaxError> {
        let installed = self.installed(inverter).await?;
        let current = self.query(inverter, &QUERY_MASK).await?;
        let (status, errors) = self.status(inverter).await?;
        if !errors.is_empty() {
            return Err(SolarMaxError::Alarm {
                inverter,
                status,
                errors,
            });
        }
        Ok(Production::from_values(&current, installed, &status)?)
    }
}
