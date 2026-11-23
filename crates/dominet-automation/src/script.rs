// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sandboxed Rhai script host (R4 lot 5, ADR 0006 and 0034): `script` actions of the automations and `handler_code` of the sources.
//!
//! A script has **no direct side effect**: `publish`, `scene` and `log_value` record [`Effect`]s that the caller
//! applies afterwards, and only if the script finished without error. `get_device` reads a snapshot supplied by the host.
//! The Rhai engine is rebuilt at each execution with strict limits (operations, depth, sizes, wall-clock time);
//! execution takes place in a `spawn_blocking` thread: a panic becomes an error, not the end of the process.
//!
//! Script variables: `topic` (text), `payload` (object or text), `device` (the device concerned or `()`).
//! A device exposes `id`, `name`, `type`, `state`, `available`, `topic` and `age_secs` (age of the last message, `()` if it never spoke).
//! Functions: `publish(topic, payload)`, `scene(name)`, `get_device(name)`, `log_value(key, value)`, `log(text)` and `print`.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rhai::packages::Package;
use rhai::{Dynamic, Engine, EvalAltResult, ParseError, Scope};
use serde_json::{Value, json};

/// Limits of an execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_operations: u64,
    pub timeout: Duration,
    /// Effects (publications, scenes, values) per execution.
    pub max_effects: usize,
    /// `print` / `log` lines kept.
    pub max_log_lines: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_operations: 200_000,
            timeout: Duration::from_millis(250),
            max_effects: 100,
            max_log_lines: 50,
        }
    }
}

/// A device as the script sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceView {
    pub id: i64,
    pub name: String,
    pub device_type: String,
    pub state: Value,
    pub available: bool,
    /// State topic (`<base>/<name>`); the script appends `/set` to command the device.
    pub topic: String,
    /// Seconds since the last message of the device, `None` if it never spoke.
    pub age_secs: Option<f64>,
}

impl DeviceView {
    /// View of a device at `now` (the age of its last message is frozen at that instant).
    #[must_use]
    pub fn of(d: &dominet_domain::device::Device, now: chrono::DateTime<chrono::Utc>) -> Self {
        Self {
            id: d.id.0,
            name: d.friendly_name.clone(),
            device_type: d.device_type.as_str().to_string(),
            state: d.state.clone(),
            available: d.available,
            topic: d.state_topic(),
            age_secs: d.last_seen.map(|t| {
                // A clock step backwards must not yield a negative age.
                let ms = (now - t).num_milliseconds().max(0);
                #[allow(clippy::cast_precision_loss)]
                let secs = ms as f64 / 1000.0;
                secs
            }),
        }
    }

    fn to_value(&self) -> Value {
        json!({"id": self.id, "name": self.name, "type": self.device_type, "state": self.state, "available": self.available,
               "topic": self.topic, "age_secs": self.age_secs})
    }
}

/// What the script receives.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScriptInput {
    pub topic: String,
    pub payload: Value,
    /// Name of the device concerned (`device` in the script), if there is one.
    pub device: Option<String>,
    /// Snapshot of the devices (for `get_device` and `device`).
    pub devices: Vec<DeviceView>,
}

/// An effect requested by the script.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// A text is published as is, any other value as JSON.
    Publish {
        topic: String,
        payload: Value,
    },
    Scene(String),
    LogValue {
        device: i64,
        key: String,
        value: Value,
    },
}

impl Effect {
    /// Readable line (dry run, log).
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Publish { topic, payload } => {
                format!("publish on {topic}: {}", payload_text(payload))
            }
            Self::Scene(name) => format!("activate scene {name}"),
            Self::LogValue { key, value, .. } => format!("log {key} = {value}"),
        }
    }
}

/// Text published for a payload: a text as is, the rest as JSON.
#[must_use]
pub fn payload_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Result of an execution.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptOutput {
    /// Empty if the script failed (nothing is applied).
    pub effects: Vec<Effect>,
    pub logs: Vec<String>,
    pub elapsed: Duration,
    pub result: Result<(), String>,
}

#[derive(Default)]
struct Collected {
    effects: Vec<Effect>,
    logs: Vec<String>,
}

type Shared = Arc<Mutex<Collected>>;

fn lock(c: &Shared) -> std::sync::MutexGuard<'_, Collected> {
    c.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn to_value(d: &Dynamic) -> Result<Value, Box<EvalAltResult>> {
    rhai::serde::from_dynamic::<Value>(d)
        .map_err(|e| format!("value cannot be converted: {e}").into())
}

fn base_engine(limits: &Limits) -> Engine {
    // `new_raw`: no implicit package; we only add the standard package (no `eval`, no module).
    let mut e = Engine::new_raw();
    e.register_global_module(rhai::packages::StandardPackage::new().as_shared_module());
    e.set_max_operations(limits.max_operations);
    e.set_max_call_levels(24);
    e.set_max_expr_depths(48, 24);
    e.set_max_string_size(16 * 1024);
    e.set_max_array_size(2_000);
    e.set_max_map_size(500);
    e.disable_symbol("eval");
    // A key absent from the payload is `()` (like `payload.get("key")` in Python): `payload.moving == "UP"` is simply false.
    e.set_fail_on_invalid_map_property(false);
    e
}

fn engine_for_run(
    limits: &Limits,
    input: &ScriptInput,
    collected: &Shared,
    deadline: Instant,
) -> Engine {
    let mut e = base_engine(limits);
    e.on_progress(move |_| {
        if Instant::now() > deadline {
            Some("timeout".into())
        } else {
            None
        }
    });
    let (c, max_lines) = (Arc::clone(collected), limits.max_log_lines);
    let log = move |s: &str| {
        let mut g = lock(&c);
        if g.logs.len() < max_lines {
            g.logs.push(s.chars().take(500).collect());
        }
    };
    let l = log.clone();
    e.on_print(move |s| l(s));
    e.on_debug(|_, _, _| {});
    e.register_fn("log", move |s: &str| log(s));

    let (c, max) = (Arc::clone(collected), limits.max_effects);
    let push = move |effect: Effect| -> Result<(), Box<EvalAltResult>> {
        let mut g = lock(&c);
        if g.effects.len() >= max {
            return Err(format!("too many effects (max {max})").into());
        }
        g.effects.push(effect);
        Ok(())
    };
    let p = push.clone();
    let publish = move |topic: &str, payload: Dynamic| -> Result<(), Box<EvalAltResult>> {
        if !crate::topic_ok(topic) {
            return Err(format!("invalid topic: {topic:?}").into());
        }
        p(Effect::Publish {
            topic: topic.to_string(),
            payload: to_value(&payload)?,
        })
    };
    e.register_fn("publish", publish.clone());
    e.register_fn("mqtt_publish", publish);
    let p = push.clone();
    e.register_fn(
        "scene",
        move |name: &str| -> Result<(), Box<EvalAltResult>> { p(Effect::Scene(name.to_string())) },
    );
    let (p, current) = (push, input.device.clone());
    let devices = input.devices.clone();
    e.register_fn(
        "log_value",
        move |key: &str, value: Dynamic| -> Result<(), Box<EvalAltResult>> {
            // Without a device concerned, nothing to record (like `PluginContext.log_value` in Python).
            let Some(id) = current
                .as_ref()
                .and_then(|n| devices.iter().find(|d| &d.name == n))
                .map(|d| d.id)
            else {
                return Ok(());
            };
            p(Effect::LogValue {
                device: id,
                key: key.to_string(),
                value: to_value(&value)?,
            })
        },
    );
    let devices = input.devices.clone();
    e.register_fn("get_device", move |name: &str| -> Dynamic {
        devices
            .iter()
            .find(|d| d.name == name)
            .and_then(|d| rhai::serde::to_dynamic(d.to_value()).ok())
            .unwrap_or(Dynamic::UNIT)
    });
    e
}

/// Readable error message (French) of an execution error.
fn friendly(e: &EvalAltResult, limits: &Limits) -> String {
    match e {
        EvalAltResult::ErrorTooManyOperations(_) => {
            format!("too many operations (maximum {})", limits.max_operations)
        }
        EvalAltResult::ErrorTerminated(..) => format!(
            "interrupted: maximum duration exceeded ({} ms)",
            limits.timeout.as_millis()
        ),
        EvalAltResult::ErrorStackOverflow(..) => "recursion too deep".to_string(),
        EvalAltResult::ErrorDataTooLarge(what, _) => format!("data too large ({what})"),
        other => other.to_string(),
    }
}

/// Runs the script. Synchronous and bounded: to be run in a thread (see [`run_isolated`]).
#[must_use]
pub fn run(code: &str, input: &ScriptInput, limits: &Limits) -> ScriptOutput {
    let started = Instant::now();
    let collected: Shared = Arc::default();
    let engine = engine_for_run(limits, input, &collected, started + limits.timeout);
    let mut scope = Scope::new();
    scope.push("topic", input.topic.clone());
    scope.push(
        "payload",
        rhai::serde::to_dynamic(&input.payload).unwrap_or(Dynamic::UNIT),
    );
    let device = input
        .device
        .as_ref()
        .and_then(|n| input.devices.iter().find(|d| &d.name == n))
        .and_then(|d| rhai::serde::to_dynamic(d.to_value()).ok())
        .unwrap_or(Dynamic::UNIT);
    scope.push_dynamic("device", device);
    let result = engine
        .run_with_scope(&mut scope, code)
        .map_err(|e| friendly(&e, limits));
    let mut g = lock(&collected);
    ScriptOutput {
        // A script in error applies nothing: all or nothing.
        effects: if result.is_ok() {
            std::mem::take(&mut g.effects)
        } else {
            vec![]
        },
        logs: std::mem::take(&mut g.logs),
        elapsed: started.elapsed(),
        result,
    }
}

/// Like [`run`], in a separate thread: a panic is an execution error.
pub async fn run_isolated(code: String, input: ScriptInput, limits: Limits) -> ScriptOutput {
    let started = Instant::now();
    tokio::task::spawn_blocking(move || run(&code, &input, &limits))
        .await
        .unwrap_or_else(|e| ScriptOutput {
            effects: vec![],
            logs: vec![],
            elapsed: started.elapsed(),
            result: Err(format!("the script made its thread fail: {e}")),
        })
}

fn describe_parse_error(e: &ParseError) -> String {
    e.to_string()
}

/// Checks the syntax (compilation only, same depth limits as execution).
///
/// # Errors
/// The parser's message, with the line and column.
pub fn check(code: &str) -> Result<(), String> {
    if looks_like_python(code) {
        return Err("this code looks like Python (def, import, trailing colon, `context.`...): scripts are written in Rhai, see the documentation".to_string());
    }
    base_engine(&Limits::default())
        .compile(code)
        .map(|_| ())
        .map_err(|e| describe_parse_error(&e))
}

/// Rough recognition of the old Python code, to refuse with a clear message rather than an obscure parse error.
#[must_use]
pub fn looks_like_python(code: &str) -> bool {
    code.lines().map(str::trim).any(|l| {
        l.starts_with("def ")
            || l.starts_with("import ")
            || (l.starts_with("from ") && l.contains(" import "))
            || l.starts_with("class ")
            || l.starts_with("elif ")
            || l.starts_with("context.")
            || l.starts_with("logger.")
            || (l.ends_with(':') && !l.starts_with("//") && !l.contains('{') && !l.contains('"'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn devices() -> Vec<DeviceView> {
        vec![
            DeviceView {
                id: 1,
                name: "Bureau".into(),
                device_type: "sensor".into(),
                state: json!({"temperature": 21.4}),
                available: true,
                topic: "zigbee2mqtt/Bureau".into(),
                age_secs: Some(0.5),
            },
            DeviceView {
                id: 2,
                name: "Salon".into(),
                device_type: "light".into(),
                state: json!({"state": "OFF"}),
                available: true,
                topic: "zigbee2mqtt/Salon".into(),
                age_secs: None,
            },
        ]
    }

    fn input(payload: Value) -> ScriptInput {
        ScriptInput {
            topic: "zigbee2mqtt/Bureau".into(),
            payload,
            device: Some("Bureau".into()),
            devices: devices(),
        }
    }

    fn run_ok(code: &str, payload: Value) -> ScriptOutput {
        let out = run(code, &input(payload), &Limits::default());
        assert_eq!(out.result, Ok(()), "{code}");
        out
    }

    #[test]
    fn a_legitimate_script_uses_the_whole_api_and_its_effects_are_returned_in_order() {
        let out = run_ok(
            r#"
            if payload.temperature > 25.0 { publish("zigbee2mqtt/Salon/set", #{state: "ON"}); }
            let d = get_device("Salon");
            log_value("bureau_temp", device.state.temperature);
            scene("Soir");
            publish("maison/texte", "bonjour");
            print(`etat ${d.state.state} sur ${topic}`);
            log("fin");
            "#,
            json!({"temperature": 27.5}),
        );
        assert_eq!(
            out.effects,
            [
                Effect::Publish {
                    topic: "zigbee2mqtt/Salon/set".into(),
                    payload: json!({"state": "ON"})
                },
                Effect::LogValue {
                    device: 1,
                    key: "bureau_temp".into(),
                    value: json!(21.4)
                },
                Effect::Scene("Soir".into()),
                Effect::Publish {
                    topic: "maison/texte".into(),
                    payload: json!("bonjour")
                },
            ]
        );
        assert_eq!(out.logs, ["etat OFF sur zigbee2mqtt/Bureau", "fin"]);
        let cold = run_ok(
            r#"if payload.temperature > 25.0 { publish("a/b", 1); }"#,
            json!({"temperature": 10}),
        );
        assert_eq!(cold.effects, []);
    }

    #[test]
    fn unknown_devices_are_unit_text_payloads_stay_text_and_other_values_become_json() {
        let out = run_ok(
            r#"
            if get_device("Unknown") == () { publish("a/b", [1, 2, #{k: true}]); }
            publish("a/c", 42);
            "#,
            json!({}),
        );
        assert_eq!(
            out.effects[0],
            Effect::Publish {
                topic: "a/b".into(),
                payload: json!([1, 2, {"k": true}])
            }
        );
        assert_eq!(payload_text(&json!("brut")), "brut");
        assert_eq!(payload_text(&json!({"a": 1})), r#"{"a":1}"#);
        assert_eq!(payload_text(&json!(42)), "42");
    }

    #[test]
    fn a_missing_key_is_unit_not_an_error() {
        let out = run_ok(
            r#"if payload.absent == () { publish("a/b", payload.n); }"#,
            json!({"n": 1}),
        );
        assert_eq!(out.effects.len(), 1);
    }

    #[test]
    fn log_value_without_a_device_does_nothing_like_python() {
        let mut i = input(json!({}));
        i.device = None;
        let out = run(
            r#"log_value("k", 1); publish("a/b", 1);"#,
            &i,
            &Limits::default(),
        );
        assert_eq!(out.result, Ok(()));
        assert_eq!(out.effects.len(), 1);
    }

    #[test]
    fn invalid_topics_and_too_many_effects_fail_the_script_and_discard_everything() {
        for code in [
            r#"publish("a/#", 1)"#,
            r#"publish("a/+/b", 1)"#,
            r#"publish("", 1)"#,
        ] {
            let out = run(
                &format!("publish(\"ok/x\", 1); {code};"),
                &input(json!({})),
                &Limits::default(),
            );
            assert!(
                out.result
                    .as_ref()
                    .is_err_and(|e| e.contains("invalid topic")),
                "{code}: {:?}",
                out.result
            );
            assert!(out.effects.is_empty(), "tout ou rien");
        }
        let out = run(
            "for i in 0..150 { publish(\"a/b\", i); }",
            &input(json!({})),
            &Limits::default(),
        );
        assert!(out.result.unwrap_err().contains("too many effects"));
        assert_eq!(out.effects, []);
    }

    #[test]
    fn hostile_scripts_are_stopped_quickly_without_effects() {
        let hostile = [
            ("boucle infinie", "loop { }"),
            ("boucle qui compte", "let i = 0; while true { i += 1; }"),
            ("recursion", "fn f(n) { f(n + 1) } f(0)"),
            ("chaine qui grossit", r#"let s = "x"; loop { s += s; }"#),
            ("tableau qui grossit", "let a = []; loop { a.push(1); }"),
            (
                "map qui grossit",
                "let m = #{}; let i = 0; loop { m[`k${i}`] = i; i += 1; }",
            ),
            ("pad enorme", r#"let s = ""; s.pad(2000000000, 'x');"#),
            ("eval", r#"eval("1 + 1")"#),
            ("effets sans fin", r#"loop { publish("a/b", 1); }"#),
        ];
        for (what, code) in hostile {
            let out = run(code, &input(json!({})), &Limits::default());
            assert!(out.result.is_err(), "{what} doit etre interrompu");
            assert!(out.effects.is_empty(), "{what}");
            assert!(
                out.elapsed < Duration::from_secs(2),
                "{what} : {:?}",
                out.elapsed
            );
        }
    }

    #[test]
    fn the_wall_clock_limit_alone_stops_a_script_with_unlimited_operations() {
        let limits = Limits {
            max_operations: 0, // unlimited
            timeout: Duration::from_millis(100),
            ..Limits::default()
        };
        let out = run("loop { }", &input(json!({})), &limits);
        assert!(out.result.unwrap_err().contains("maximum duration"));
        assert!(
            out.elapsed < Duration::from_millis(1000),
            "{:?}",
            out.elapsed
        );
    }

    #[test]
    fn imports_and_deep_nesting_are_refused_at_compilation() {
        assert!(check(r#"import "x" as y;"#).is_err());
        let deep = format!("{}1{}", "(".repeat(5000), ")".repeat(5000));
        assert!(check(&deep).is_err());
        assert!(check("let x = ;").is_err());
        assert_eq!(check(r#"let x = 1; publish("a/b", x);"#), Ok(()));
    }

    #[test]
    fn old_python_code_is_refused_with_a_clear_message() {
        for code in [
            "def f():\n    pass",
            "import json",
            "from x import y",
            "context.publish('a', 1)",
            "if payload.get('a'):\n    pass",
            "elif x:",
        ] {
            assert!(looks_like_python(code), "{code}");
            assert!(check(code).unwrap_err().contains("Python"), "{code}");
        }
        for code in [
            "let a = 1;",
            "// note: deux-points\nlet b = #{k: 1};",
            "if x { y(); }",
        ] {
            assert!(!looks_like_python(code), "{code}");
        }
    }

    #[tokio::test]
    async fn a_panic_in_the_script_thread_is_an_error_not_a_crash() {
        let out = tokio::task::spawn_blocking(|| -> ScriptOutput { panic!("boum") })
            .await
            .map_err(|e| e.to_string());
        assert!(out.is_err(), "la panique est rattrapee par le thread");
        let ok = run_isolated(
            "publish(\"a/b\", 1);".into(),
            input(json!({})),
            Limits::default(),
        )
        .await;
        assert_eq!(ok.result, Ok(()));
        assert_eq!(ok.effects.len(), 1);
    }
    #[test]
    fn a_device_exposes_its_topic_and_the_age_of_its_last_message() {
        let out = run(
            r#"
            let b = get_device("Bureau");
            if b.age_secs < 2.0 { publish(b.topic + "/set", "recent"); }
            let s = get_device("Salon");
            if s.age_secs == () { publish(s.topic + "/set", "never"); }
            if device.age_secs >= 2 { publish("a/b", "old"); }
            "#,
            &input(json!({})),
            &Limits::default(),
        );
        assert_eq!(out.result, Ok(()));
        assert_eq!(
            out.effects,
            [
                Effect::Publish {
                    topic: "zigbee2mqtt/Bureau/set".into(),
                    payload: json!("recent")
                },
                Effect::Publish {
                    topic: "zigbee2mqtt/Salon/set".into(),
                    payload: json!("never")
                },
            ]
        );
    }
}
