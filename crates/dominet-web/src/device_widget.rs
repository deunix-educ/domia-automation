// SPDX-License-Identifier: AGPL-3.0-or-later
//! Body of a device widget: live state (labels, units and hidden keys from the devices table), active commands
//! (R3 lot 5, ADR 0029). The commands take over those of the reference implementation (`{"state":"ON"}`, `{"brightness":n}`,
//! `{"occupied_heating_setpoint":x}`, `{"fan_mode":"low"}`...); the effect is seen when the device republishes its state.

use std::str::FromStr;

use dominet_domain::command::MAX_COMMAND_BYTES;
use dominet_domain::dashboard::Presentation;
use dominet_domain::device::DeviceType;
use dominet_domain::widget_view::{StateRow, presentation_for, state_rows_with};
use dominet_proto::{CommandOutcome, DeviceMetaDto, DeviceStateDto, WidgetDto};
use leptos::prelude::*;
use leptos_fluent::{move_tr, tr};
use serde_json::{Value, json};

use crate::device_fns::send_device_command;

/// What a widget's buttons need to command its device.
#[derive(Clone, Copy)]
struct Cmd {
    action: Action<String, Result<CommandOutcome, ServerFnError>>,
    /// True if the server can publish AND the device is available (otherwise "source unavailable").
    enabled: Signal<bool>,
}

impl Cmd {
    fn send(self, command: &Value) {
        let text = command.to_string();
        debug_assert!(text.len() <= MAX_COMMAND_BYTES);
        self.action.dispatch(text);
    }
}

pub(crate) fn device_body(w: &WidgetDto, state: Signal<Option<DeviceStateDto>>) -> AnyView {
    let requested = Presentation::from_str(&w.presentation).unwrap_or(Presentation::Auto);
    let title = w.title.clone();
    let meta = w.device_meta.clone().unwrap_or_default();
    let device_id = w.device_id.unwrap_or_default();
    let commandable = meta.commandable;
    let action = Action::new(move |command: &String| {
        let command = command.clone();
        async move { send_device_command(device_id, command).await }
    });
    let cmd = Cmd {
        action,
        enabled: Signal::derive(move || commandable && state.get().is_some_and(|s| s.available)),
    };
    // Message of the last refusal: it clears as soon as the device publishes a new state (so the failure is over or outdated).
    let refusal = RwSignal::new(None::<&'static str>);
    Effect::new(move |_| {
        refusal.set(match action.value().get() {
            None | Some(Ok(CommandOutcome::Sent)) => None,
            Some(Ok(CommandOutcome::Unavailable)) => Some(tk!("dev-cmd-unavailable")),
            Some(Ok(_)) => Some(tk!("dev-cmd-refused")),
            Some(Err(_)) => Some(tk!("dev-cmd-failed")),
        });
    });
    Effect::new(move |previous: Option<()>| {
        state.track();
        if previous.is_some() {
            refusal.set(None);
        }
    });
    let i18n = expect_context::<leptos_fluent::I18n>();
    let message = move || refusal.get().map(|key| i18n.tr(key)).unwrap_or_default();
    view! {
        <div class="vc-widget-body--dev" id=format!("dev-body-{}", w.id)>
            {move || match state.get() {
                None => view! { <p class="vc-muted" id="dev-waiting">{move_tr!("dev-waiting")}</p> }.into_any(),
                Some(s) => device_view(requested, &title, &s, &meta, cmd),
            }}
            <div class="vc-dev-msg vc-muted" id=format!("dev-msg-{}", w.id) role="status">{message}</div>
        </div>
    }
    .into_any()
}

fn humanize(key: &str) -> String {
    let spaced = key.replace('_', " ");
    let mut chars = spaced.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}

/// Displayed name of a row: the device's label, otherwise the formatted key.
fn row_name(r: &StateRow) -> String {
    if r.name.is_empty() {
        humanize(&r.key)
    } else {
        r.name.clone()
    }
}

fn num(state: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .find_map(|k| state.get(*k).and_then(Value::as_f64))
}

fn text_of(state: &Value, key: &str) -> Option<String> {
    state.get(key).and_then(Value::as_str).map(str::to_string)
}

fn value_cell(r: &StateRow) -> AnyView {
    if r.is_bool {
        let on = r.on;
        view! { <span>{move || if on { tr!("dev-yes") } else { tr!("dev-no") }}</span> }.into_any()
    } else if r.unit.is_empty() {
        view! { <span>{r.value.clone()}</span> }.into_any()
    } else {
        view! { <span>{r.value.clone()}"\u{a0}"<small>{r.unit.clone()}</small></span> }.into_any()
    }
}

fn state_grid(rows: &[StateRow]) -> AnyView {
    if rows.is_empty() {
        return view! { <div class="vc-dev-state"><span class="vc-dev-state-k">"—"</span></div> }
            .into_any();
    }
    view! {
        <div class="vc-dev-state">
            {rows
                .iter()
                .map(|r| {
                    view! {
                        <span class="vc-dev-state-k">{row_name(r)}</span>
                        <span class="vc-dev-state-v">{value_cell(r)}</span>
                    }
                })
                .collect_view()}
        </div>
    }
    .into_any()
}

/// Command button: active if the server can publish and the device is available.
fn control(label: AnyView, on: bool, cmd: Cmd, command: Value) -> AnyView {
    view! {
        <button
            type="button"
            class="vc-dev-ctrl"
            class:vc-dev-ctrl--on=on
            disabled=move || !cmd.enabled.get()
            title=move || (!cmd.enabled.get()).then(|| tr!("dev-source-unavailable"))
            on:click=move |_| cmd.send(&command)
        >
            {label}
        </button>
    }
    .into_any()
}

fn footer(buttons: Vec<AnyView>) -> AnyView {
    view! { <div class="vc-dev-footer">{buttons}</div> }.into_any()
}

/// Slider: the `{key: value}` command goes out on release.
fn slider(
    label: AnyView,
    id: &'static str,
    key: &'static str,
    value: f64,
    max: u32,
    cmd: Cmd,
) -> AnyView {
    view! {
        <div class="vc-dev-slider-row">
            <span class="vc-dev-slider-lbl">{label}</span>
            <input
                type="range"
                class="vc-dev-slider"
                id=id
                min="0"
                max=max
                value=value.to_string()
                disabled=move || !cmd.enabled.get()
                on:change=move |ev| {
                    if let Ok(v) = event_target_value(&ev).parse::<i64>() {
                        cmd.send(&json!({ key: v }));
                    }
                }
            />
            <span class="vc-dev-slider-val">{format!("{value:.0}")}</span>
        </div>
    }
    .into_any()
}

fn primary(row: &StateRow) -> AnyView {
    view! {
        <div class="vc-dev-primary" id="dev-primary">
            <span class="vc-dev-primary-key">{row_name(row)}</span>
            <div class="vc-dev-primary-line">
                <span class="vc-dev-primary-val">{row.value.clone()}</span>
                {(!row.unit.is_empty()).then(|| view! { <span class="vc-dev-primary-unit">{row.unit.clone()}</span> })}
            </div>
        </div>
    }
    .into_any()
}

fn on_off(state: &Value, cmd: Cmd) -> AnyView {
    let known = state.get("state").is_some();
    let on = state.get("state").and_then(Value::as_str) == Some("ON");
    footer(vec![
        control(view! { "ON" }.into_any(), on, cmd, json!({"state": "ON"})),
        control(
            view! { "OFF" }.into_any(),
            known && !on,
            cmd,
            json!({"state": "OFF"}),
        ),
    ])
}

fn sensor_view(rows: &[StateRow]) -> AnyView {
    let main: Vec<&StateRow> = rows.iter().filter(|r| !is_diagnostic(&r.key)).collect();
    if let [only] = main.as_slice() {
        primary(only)
    } else {
        state_grid(rows)
    }
}

fn is_diagnostic(key: &str) -> bool {
    matches!(
        key,
        "linkquality" | "battery" | "voltage" | "update" | "update_available" | "last_seen"
    )
}

fn badges_view(rows: &[StateRow]) -> AnyView {
    view! {
        <div class="vc-dev-badges" id="dev-badges">
            {rows
                .iter()
                .map(|r| view! { <span class="vc-dev-badge" class:vc-dev-badge--on=r.on>{row_name(r)}</span> })
                .collect_view()}
        </div>
    }
    .into_any()
}

fn light_view(st: &Value, rows: &[StateRow], cmd: Cmd) -> AnyView {
    let mut parts = vec![state_grid(rows)];
    if let Some(b) = num(st, &["brightness"]) {
        parts.push(slider(
            move_tr!("dev-brightness").into_any(),
            "dev-brightness",
            "brightness",
            b,
            254,
            cmd,
        ));
    }
    if let Some(t) = num(st, &["color_temp"]) {
        parts.push(slider(
            move_tr!("dev-color-temp").into_any(),
            "dev-color-temp",
            "color_temp",
            t,
            500,
            cmd,
        ));
    }
    parts.push(on_off(st, cmd));
    parts.into_any()
}

fn cover_view(st: &Value, rows: &[StateRow], cmd: Cmd) -> AnyView {
    let mut parts = vec![state_grid(rows)];
    if let Some(p) = num(st, &["position"]) {
        parts.push(slider(
            move_tr!("dev-position").into_any(),
            "dev-position",
            "position",
            p,
            100,
            cmd,
        ));
    }
    parts.push(footer(vec![
        control(
            move_tr!("dev-open").into_any(),
            false,
            cmd,
            json!({"state": "OPEN"}),
        ),
        control(
            move_tr!("dev-stop").into_any(),
            false,
            cmd,
            json!({"state": "STOP"}),
        ),
        control(
            move_tr!("dev-close").into_any(),
            false,
            cmd,
            json!({"state": "CLOSE"}),
        ),
    ]));
    parts.into_any()
}

/// Setpoint shifted by `delta`, rounded to the half degree (like the reference).
fn next_setpoint(current: Option<f64>, delta: f64) -> f64 {
    ((current.unwrap_or(20.0) + delta) * 2.0).round() / 2.0
}

fn climate_view(st: &Value, rows: &[StateRow], cmd: Cmd) -> AnyView {
    let current = num(st, &["local_temperature", "temperature"]);
    let setpoint = num(
        st,
        &["occupied_heating_setpoint", "current_heating_setpoint"],
    );
    let step = |delta: f64| json!({"occupied_heating_setpoint": next_setpoint(setpoint, delta)});
    view! {
        <div class="vc-dev-climate" id="dev-climate">
            <div>
                <span class="vc-dev-climate-big">{current.map_or_else(|| "—".to_string(), |c| format!("{c:.1}"))}</span>
                <span class="vc-dev-climate-unit">"°C"</span>
            </div>
            <div class="vc-dev-climate-sp">
                {control(view! { "−" }.into_any(), false, cmd, step(-0.5))}
                <span id="dev-setpoint">{setpoint.map_or_else(|| "—".to_string(), |c| format!("{c:.1} °C"))}</span>
                {control(view! { "+" }.into_any(), false, cmd, step(0.5))}
            </div>
        </div>
        {state_grid(rows)}
    }
    .into_any()
}

fn fan_view(st: &Value, rows: &[StateRow], cmd: Cmd) -> AnyView {
    let mode = text_of(st, "fan_mode").unwrap_or_default();
    let button = |value: &'static str, label: AnyView| {
        control(label, mode == value, cmd, json!({"fan_mode": value}))
    };
    view! {
        {state_grid(rows)}
        {footer(vec![
            button("off", move_tr!("dev-fan-off").into_any()),
            button("low", move_tr!("dev-fan-low").into_any()),
            button("medium", move_tr!("dev-fan-medium").into_any()),
            button("high", move_tr!("dev-fan-high").into_any()),
            button("auto", move_tr!("dev-fan-auto").into_any()),
        ])}
    }
    .into_any()
}

fn lock_view(st: &Value, rows: &[StateRow], cmd: Cmd) -> AnyView {
    let locked = text_of(st, "state")
        .or_else(|| text_of(st, "lock_state"))
        .unwrap_or_default();
    view! {
        <div class="vc-dev-primary" id="dev-lock">
            <span class="vc-dev-primary-val">{locked}</span>
        </div>
        {state_grid(rows)}
        {footer(vec![
            control(move_tr!("dev-lock").into_any(), false, cmd, json!({"state": "LOCK"})),
            control(move_tr!("dev-unlock").into_any(), false, cmd, json!({"state": "UNLOCK"})),
        ])}
    }
    .into_any()
}

fn weather_view(st: &Value, rows: &[StateRow]) -> AnyView {
    let temp = rows.iter().find(|r| r.key == "temperature");
    let others: Vec<StateRow> = rows
        .iter()
        .filter(|r| r.key != "temperature" && r.key != "condition")
        .cloned()
        .collect();
    view! {
        {temp.map(primary)}
        {text_of(st, "condition").map(|c| view! { <div class="vc-dev-state-v" id="dev-condition">{c}</div> })}
        {state_grid(&others)}
        <p class="vc-muted" id="dev-weather-attribution">
            <a href="https://open-meteo.com/" target="_blank" rel="noopener">{move_tr!("weather-attribution")}</a>
        </p>
    }
    .into_any()
}

/// Body of a device widget according to the presentation (requested, or deduced from the type then from the state if automatic).
fn device_view(
    requested: Presentation,
    title: &str,
    s: &DeviceStateDto,
    meta: &DeviceMetaDto,
    cmd: Cmd,
) -> AnyView {
    use Presentation as P;
    let st = &s.state;
    let labels: Vec<(String, String, String)> = meta
        .labels
        .iter()
        .map(|l| (l.key.clone(), l.name.clone(), l.unit.clone()))
        .collect();
    let rows = state_rows_with(st, &labels, &meta.hidden_keys);
    let name = (s.friendly_name != title && !s.friendly_name.is_empty()).then(|| {
        view! { <div class="vc-dev-name" title=s.friendly_name.clone()>{s.friendly_name.clone()}</div> }
    });
    let kind = DeviceType::from_str(&meta.device_type).ok();
    let body = match presentation_for(requested, kind, st) {
        P::DeviceSensor => sensor_view(&rows),
        P::DeviceBinarySensor => badges_view(&rows),
        P::DeviceSwitch => view! { {state_grid(&rows)} {on_off(st, cmd)} }.into_any(),
        P::DeviceLight => light_view(st, &rows, cmd),
        P::DeviceCover => cover_view(st, &rows, cmd),
        P::DeviceClimate => climate_view(st, &rows, cmd),
        P::DeviceFan => fan_view(st, &rows, cmd),
        P::DeviceLock => lock_view(st, &rows, cmd),
        P::DeviceWeather => weather_view(st, &rows),
        _ => state_grid(&rows),
    };
    view! { {name} {body} }.into_any()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setpoint_moves_by_half_degrees() {
        assert_eq!(next_setpoint(Some(20.0), 0.5), 20.5);
        assert_eq!(next_setpoint(Some(20.5), -0.5), 20.0);
        assert_eq!(next_setpoint(Some(19.3), 0.5), 20.0);
        assert_eq!(next_setpoint(Some(19.3), -0.5), 19.0);
        assert_eq!(next_setpoint(None, 0.5), 20.5);
    }

    #[test]
    fn keys_are_humanized_when_no_label_exists() {
        assert_eq!(humanize("color_temp"), "Color temp");
        assert_eq!(humanize(""), "");
    }
}
