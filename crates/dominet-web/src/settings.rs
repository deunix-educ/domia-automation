// SPDX-License-Identifier: AGPL-3.0-or-later
//! Administration of application settings: `/admin/settings`.
//!
//! The 17 settings are fixed (domain registry): no creation or deletion, hence no CRUD table.
//! One card per category, an editor suited to the type. A secret's value never reaches the browser: we
//! only show whether it is defined, and only send it when it is replaced.

use dominet_proto::{BroadcastOutcome, InjectOutcome, SettingDto, SettingOutcome, Topic};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_fluent::{move_tr, tr};

use crate::guard::RequireAdmin;
use crate::layout::use_hydrated;
use crate::realtime::Realtime;
use crate::server_fns::{broadcast_notification, inject_device_state, list_settings, save_setting};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Saved,
    Invalid,
    NoMasterKey,
    Unknown,
    Failed,
}

/// Settings grouped by category, in the order received (already sorted by category then key).
fn by_category(items: Vec<SettingDto>) -> Vec<(String, Vec<SettingDto>)> {
    let mut out: Vec<(String, Vec<SettingDto>)> = vec![];
    for item in items {
        match out.last_mut() {
            Some((cat, group)) if *cat == item.category => group.push(item),
            _ => out.push((item.category.clone(), vec![item])),
        }
    }
    out
}

#[component]
pub fn SettingsPage() -> impl IntoView {
    view! {
        <RequireAdmin>
            <main class="vc-container">
                <SettingsAdmin/>
            </main>
        </RequireAdmin>
    }
}

#[component]
fn SettingsAdmin() -> impl IntoView {
    let list = Resource::new(|| (), |()| async { list_settings().await.map_err(|_| ()) });
    view! {
        <div class="vc-page-header">
            <h2>{move_tr!("settings-title")}</h2>
        </div>
        <p class="vc-muted">{move_tr!("settings-intro")}</p>
        <BroadcastCard/>
        <SimulatorCard/>
        <Transition fallback=|| ()>
            {move || {
                list.get()
                    .map(|res| match res {
                        Err(()) => {
                            view! { <div class="vc-alert vc-alert--error" role="alert" id="crud-load-error">{move_tr!("crud-load-error")}</div> }
                                .into_any()
                        }
                        Ok(items) => {
                            by_category(items)
                                .into_iter()
                                .map(|(category, group)| {
                                    view! {
                                        <section class="vc-card">
                                            <h3 class="vc-card-title">{category}</h3>
                                            {group.into_iter().map(|s| view! { <SettingRow initial=s/> }).collect_view()}
                                        </section>
                                    }
                                })
                                .collect_view()
                                .into_any()
                        }
                    })
            }}
        </Transition>
    }
}

/// A setting: description, state, editor and buttons.
#[component]
fn SettingRow(initial: SettingDto) -> impl IntoView {
    let key = initial.key.clone();
    let value_type = initial.value_type.clone();
    let is_secret = initial.is_secret;
    let current = RwSignal::new(initial.clone());
    // A secret is never prefilled; the others are, with the current value.
    let draft = RwSignal::new(if is_secret {
        String::new()
    } else {
        initial.value.clone()
    });
    let status = RwSignal::new(None::<Status>);
    let pending = RwSignal::new(false);
    let ready = use_hydrated();

    let send = {
        let key = key.clone();
        move |value: String| {
            if pending.get_untracked() {
                return;
            }
            pending.set(true);
            status.set(None);
            let key = key.clone();
            spawn_local(async move {
                match save_setting(key, value).await {
                    Ok(SettingOutcome::Saved(dto)) => {
                        draft.set(if dto.is_secret {
                            String::new()
                        } else {
                            dto.value.clone()
                        });
                        current.set(dto);
                        status.set(Some(Status::Saved));
                    }
                    Ok(SettingOutcome::Invalid) => status.set(Some(Status::Invalid)),
                    Ok(SettingOutcome::MasterKeyMissing) => status.set(Some(Status::NoMasterKey)),
                    Ok(SettingOutcome::UnknownKey) => status.set(Some(Status::Unknown)),
                    Err(_) => status.set(Some(Status::Failed)),
                }
                pending.set(false);
            });
        }
    };
    let on_submit = {
        let send = send.clone();
        move |ev: leptos::ev::SubmitEvent| {
            ev.prevent_default();
            send(draft.get_untracked());
        }
    };
    let on_clear = {
        let send = send.clone();
        move |_| send(String::new())
    };

    let input_id = format!("setting-{key}");
    let editor = match (value_type.as_str(), is_secret) {
        (_, true) => view! {
            <input
                id=input_id.clone()
                type="password"
                autocomplete="new-password"
                placeholder=move || tr!("settings-secret-placeholder")
                prop:value=move || draft.get()
                on:input=move |ev| draft.set(event_target_value(&ev))
            />
        }
        .into_any(),
        ("int", _) => view! {
            <input id=input_id.clone() type="number" step="1" required value=draft.get_untracked() prop:value=move || draft.get()
                on:input=move |ev| draft.set(event_target_value(&ev))/>
        }
        .into_any(),
        ("float", _) => view! {
            <input id=input_id.clone() type="number" step="any" required value=draft.get_untracked() prop:value=move || draft.get()
                on:input=move |ev| draft.set(event_target_value(&ev))/>
        }
        .into_any(),
        ("bool", _) => view! {
            <input id=input_id.clone() type="checkbox" prop:checked=move || draft.get() == "true"
                on:change=move |ev| draft.set(event_target_checked(&ev).to_string())/>
        }
        .into_any(),
        _ => view! {
            <input id=input_id.clone() type="text" value=draft.get_untracked() prop:value=move || draft.get()
                on:input=move |ev| draft.set(event_target_value(&ev))/>
        }
        .into_any(),
    };

    let (save_id, status_id, clear_id) = (
        format!("setting-save-{key}"),
        format!("setting-status-{key}"),
        format!("setting-clear-{key}"),
    );
    let type_for_message = value_type.clone();
    view! {
        <form method="post" class="vc-setting" on:submit=on_submit data-key=key.clone()>
            <div class="vc-setting-head">
                <code>{key.clone()}</code>
                <span class="vc-badge vc-badge--info">{value_type.clone()}</span>
                {is_secret.then(|| view! { <span class="vc-badge vc-badge--warning">{move_tr!("settings-secret")}</span> })}
                {move || {
                    let c = current.get();
                    if is_secret {
                        let (class, label) = if !c.readable {
                            ("vc-badge vc-badge--danger", tr!("settings-unreadable"))
                        } else if c.is_set {
                            ("vc-badge vc-badge--success", tr!("settings-set"))
                        } else {
                            ("vc-badge", tr!("settings-unset"))
                        };
                        Some(view! { <span class=class id=format!("setting-state-{}", c.key)>{label}</span> })
                    } else if c.from_default {
                        Some(view! { <span class="vc-badge" id=format!("setting-state-{}", c.key)>{tr!("settings-default")}</span> })
                    } else {
                        None
                    }
                }}
            </div>
            <p class="vc-muted">{initial.description.clone()}</p>
            <div class="vc-setting-edit">
                {editor}
                <button type="submit" id=save_id class="vc-btn vc-btn--primary vc-btn--sm" disabled=move || !ready.get() || pending.get()>
                    {move_tr!("crud-save")}
                </button>
                {is_secret.then(|| {
                    view! {
                        <button type="button" id=clear_id class="vc-btn vc-btn--danger vc-btn--sm"
                            disabled=move || !ready.get() || pending.get() || !current.get().is_set
                            on:click=on_clear.clone()>
                            {move_tr!("settings-clear")}
                        </button>
                    }
                })}
            </div>
            {move || {
                status
                    .get()
                    .map(|s| {
                        let class = if s == Status::Saved { "vc-alert vc-alert--info" } else { "vc-alert vc-alert--error" };
                        let ty = type_for_message.clone();
                        view! {
                            <div class=class role="status" id=status_id.clone()>
                                {move || match s {
                                    Status::Saved => tr!("settings-saved"),
                                    Status::Invalid => invalid_text(&ty),
                                    Status::NoMasterKey => tr!("settings-master-key-missing"),
                                    Status::Unknown => tr!("settings-unknown"),
                                    Status::Failed => tr!("crud-error-failed"),
                                }}
                            </div>
                        }
                    })
            }}
        </form>
    }
}

/// Message for a value incompatible with the setting's type.
fn invalid_text(value_type: &str) -> String {
    match value_type {
        "int" => tr!("settings-invalid-int"),
        "float" => tr!("settings-invalid-float"),
        "bool" => tr!("settings-invalid-bool"),
        _ => tr!("crud-invalid"),
    }
}

/// Broadcast of a message to all the logged-in users: useful and serves to check the real-time chain.
#[component]
fn BroadcastCard() -> impl IntoView {
    let text = RwSignal::new(String::new());
    let pending = RwSignal::new(false);
    let result = RwSignal::new(None::<Option<u32>>);
    let ready = use_hydrated();
    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        if pending.get_untracked() {
            return;
        }
        pending.set(true);
        let message = text.get_untracked();
        spawn_local(async move {
            match broadcast_notification(message).await {
                Ok(BroadcastOutcome::Sent { receivers }) => {
                    result.set(Some(Some(receivers)));
                    text.set(String::new());
                }
                Ok(BroadcastOutcome::Invalid) | Err(_) => result.set(Some(None)),
            }
            pending.set(false);
        });
    };
    view! {
        <section class="vc-card" id="broadcast">
            <h3 class="vc-card-title">{move_tr!("settings-broadcast-title")}</h3>
            <p class="vc-muted">{move_tr!("settings-broadcast-hint")}</p>
            <form method="post" on:submit=on_submit class="vc-setting-edit">
                <input
                    id="broadcast-text"
                    type="text"
                    maxlength="200"
                    required
                    placeholder=move || tr!("settings-broadcast-placeholder")
                    prop:value=move || text.get()
                    on:input=move |ev| text.set(event_target_value(&ev))
                />
                <button type="submit" id="broadcast-send" class="vc-btn vc-btn--primary vc-btn--sm" disabled=move || !ready.get() || pending.get()>
                    {move_tr!("settings-broadcast-send")}
                </button>
            </form>
            {move || {
                result
                    .get()
                    .map(|r| {
                        view! {
                            <div class="vc-alert vc-alert--info" role="status" id="broadcast-status">
                                {match r {
                                    Some(n) => tr!("settings-broadcast-sent", {"count" => n}),
                                    None => tr!("crud-error-failed"),
                                }}
                            </div>
                        }
                    })
            }}
        </section>
    }
}

/// Result of a test injection, as displayed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Injected {
    Sent(u32),
    InvalidDevice,
    InvalidState,
    Failed,
}

/// Injection of a test device state and live display of the last state received for this number: the
/// subscription -> kept state -> broadcast chain is checked here before MQTT exists (R3).
#[component]
fn SimulatorCard() -> impl IntoView {
    let rt = expect_context::<Realtime>();
    let device = RwSignal::new(String::new());
    let state = RwSignal::new(r#"{"state": "ON", "temperature": 21.5}"#.to_string());
    let available = RwSignal::new(true);
    let pending = RwSignal::new(false);
    let result = RwSignal::new(None::<Injected>);
    let ready = use_hydrated();
    let id = move || device.get().trim().parse::<i64>().ok().filter(|n| *n > 0);

    // We follow the device whose number is entered: subscription taken, changed or given up with the field.
    let followed = StoredValue::new(None::<i64>);
    Effect::new(move |_| {
        let current = id();
        let previous = followed.get_value();
        if previous != current {
            if let Some(old) = previous {
                rt.unsubscribe(&Topic::Device(old));
            }
            if let Some(new) = current {
                rt.subscribe(Topic::Device(new));
            }
            followed.set_value(current);
        }
    });
    on_cleanup(move || {
        if let Some(old) = followed.get_value() {
            rt.unsubscribe(&Topic::Device(old));
        }
    });

    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        if pending.get_untracked() {
            return;
        }
        let Some(number) = id() else {
            result.set(Some(Injected::InvalidDevice));
            return;
        };
        pending.set(true);
        let (text, ok) = (state.get_untracked(), available.get_untracked());
        spawn_local(async move {
            result.set(Some(match inject_device_state(number, text, ok).await {
                Ok(InjectOutcome::Sent { receivers }) => Injected::Sent(receivers),
                Ok(InjectOutcome::InvalidDevice) => Injected::InvalidDevice,
                Ok(InjectOutcome::InvalidState) => Injected::InvalidState,
                Err(_) => Injected::Failed,
            }));
            pending.set(false);
        });
    };
    view! {
        <section class="vc-card" id="simulator">
            <h3 class="vc-card-title">{move_tr!("settings-sim-title")}</h3>
            <p class="vc-muted">{move_tr!("settings-sim-hint")}</p>
            <form method="post" on:submit=on_submit>
                <div class="vc-form-group">
                    <label for="sim-device">{move_tr!("settings-sim-device")}</label>
                    <input
                        id="sim-device"
                        type="number"
                        min="1"
                        step="1"
                        value=device.get_untracked()
                        prop:value=move || device.get()
                        on:input=move |ev| device.set(event_target_value(&ev))
                    />
                </div>
                <div class="vc-form-group">
                    <label for="sim-state">{move_tr!("settings-sim-state")}</label>
                    <textarea
                        id="sim-state"
                        rows="3"
                        prop:value=move || state.get()
                        on:input=move |ev| state.set(event_target_value(&ev))
                    >
                        {state.get_untracked()}
                    </textarea>
                </div>
                <div class="vc-form-group vc-form-group--check">
                    <label for="sim-available">{move_tr!("settings-sim-available")}</label>
                    <input
                        id="sim-available"
                        type="checkbox"
                        checked
                        prop:checked=move || available.get()
                        on:change=move |ev| available.set(event_target_checked(&ev))
                    />
                </div>
                <div class="vc-form-actions">
                    <button
                        type="submit"
                        id="sim-send"
                        class="vc-btn vc-btn--primary vc-btn--sm"
                        disabled=move || !ready.get() || pending.get()
                    >
                        {move_tr!("settings-sim-send")}
                    </button>
                </div>
            </form>
            {move || {
                result
                    .get()
                    .map(|r| {
                        let class = if matches!(r, Injected::Sent(_)) {
                            "vc-alert vc-alert--info"
                        } else {
                            "vc-alert vc-alert--error"
                        };
                        view! {
                            <div class=class role="status" id="sim-status">
                                {match r {
                                    Injected::Sent(n) => tr!("settings-sim-sent", {"count" => n}),
                                    Injected::InvalidDevice => tr!("settings-sim-invalid-device"),
                                    Injected::InvalidState => tr!("settings-sim-invalid-state"),
                                    Injected::Failed => tr!("crud-error-failed"),
                                }}
                            </div>
                        }
                    })
            }}
            <h4 class="vc-card-title">{move_tr!("settings-sim-received")}</h4>
            <pre id="sim-received" class="vc-muted">
                {move || match id().and_then(|n| rt.device(n)) {
                    Some(d) => {
                        format!("{} | {} | {}", d.friendly_name, d.available, d.state)
                    }
                    None => tr!("settings-sim-none"),
                }}
            </pre>
        </section>
    }
}
