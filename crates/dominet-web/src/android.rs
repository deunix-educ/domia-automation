// SPDX-License-Identifier: AGPL-3.0-or-later
//! Android pages (R5 lot 4, ADR 0040): `/android` dashboard, `/android/devices` devices, `/android/sms` SMS and the call log
//! `/android/calls`. Every account sees its devices, an administrator sees all the devices (like the reference). The pages
//! update live through the WebSocket `android:<id>` subjects.

use dominet_proto::{
    AndroidCallsPageDto, AndroidDeviceDto, AndroidDeviceInput, AndroidOverviewDto,
    AndroidSmsPageDto, CallDto, DeleteOutcome, FieldProblem, NewAndroidDeviceDto, SaveOutcome,
    SendSmsOutcome, SmsDto, Topic,
};
use leptos::prelude::*;
use leptos_fluent::{move_tr, tr};
use leptos_router::components::A;
use leptos_router::hooks::use_query_map;

use crate::browser::{clear_input, input_value};
use crate::crud::field_error;
use crate::guard::RequireAuth;
use crate::layout::use_hydrated;
use crate::realtime::Realtime;

// ------------------------------------------------------------------ server functions

#[cfg(feature = "ssr")]
mod ssr {
    use chrono::{DateTime, Utc};
    use dominet_domain::android::{AndroidDevice, AndroidDeviceId};
    use dominet_domain::repo::AndroidDeviceRepo;
    use dominet_domain::user::{User, UserId};
    use dominet_proto::AndroidDeviceDto;
    use leptos::prelude::ServerFnError;

    use crate::server_fns::internal;

    pub fn time(t: DateTime<Utc>) -> String {
        dominet_server::timefmt::rfc3339(t)
    }

    /// What this account sees: its devices, or all of them for an administrator (`None`).
    pub fn scope(user: &User) -> Option<UserId> {
        (!user.role.is_admin()).then_some(user.id)
    }

    pub async fn devices(
        state: &dominet_server::AppState,
        user: &User,
    ) -> Result<Vec<AndroidDevice>, ServerFnError> {
        state
            .store
            .android_devices()
            .list_for_owner(scope(user))
            .await
            .map_err(|e| internal("Android devices", &e))
    }

    /// The device, only if it belongs to this account (or if it is an administrator): otherwise, as if it did not exist.
    pub async fn device(
        state: &dominet_server::AppState,
        user: &User,
        id: i64,
    ) -> Result<Option<AndroidDevice>, ServerFnError> {
        let found = state
            .store
            .android_devices()
            .get(AndroidDeviceId(id))
            .await
            .map_err(|e| internal("Android device", &e))?;
        Ok(found.filter(|d| d.owner == user.id || user.role.is_admin()))
    }

    pub async fn dtos(
        state: &dominet_server::AppState,
        devices: &[AndroidDevice],
    ) -> Result<Vec<AndroidDeviceDto>, ServerFnError> {
        use dominet_domain::repo::UserRepo;
        let names: std::collections::HashMap<i64, String> = state
            .store
            .users()
            .list()
            .await
            .map_err(|e| internal("accounts", &e))?
            .into_iter()
            .map(|u| (u.id.0, u.username))
            .collect();
        Ok(devices
            .iter()
            .map(|d| AndroidDeviceDto {
                id: d.id.0,
                name: d.name.clone(),
                phone_number: d.phone_number.clone(),
                owner: names.get(&d.owner.0).cloned().unwrap_or_default(),
                is_online: d.is_online,
                last_seen: d.last_seen.map(time).unwrap_or_default(),
            })
            .collect())
    }

    /// SMS with the sending state of those that leave from the browser.
    pub async fn sms_dtos(
        state: &dominet_server::AppState,
        sms: &[dominet_domain::android::SmsMessage],
    ) -> Result<Vec<dominet_proto::SmsDto>, ServerFnError> {
        use dominet_domain::repo::SmsOutboxRepo;
        let ids: Vec<_> = sms.iter().map(|s| s.id).collect();
        let outbox = state
            .store
            .sms_outbox()
            .for_sms(&ids)
            .await
            .map_err(|e| internal("SMS send state", &e))?;
        Ok(sms
            .iter()
            .map(|s| {
                let out = outbox.iter().find(|o| {
                    matches!(o.origin, dominet_domain::android::OutboxOrigin::Browser(id) if id == s.id)
                });
                dominet_server::android::sms_dto(s, out)
            })
            .collect())
    }
}

/// Dashboard: devices, last 10 SMS and calls, unread SMS.
///
/// # Errors
/// Refused (401) without a session; internal error if the database fails.
#[server(name = AndroidOverview, prefix = "/sf", endpoint = "android/overview")]
pub async fn android_overview() -> Result<AndroidOverviewDto, ServerFnError> {
    use dominet_domain::repo::{CallRepo, SmsRepo};

    use crate::server_fns::{internal, require_user};

    let state = expect_context::<dominet_server::AppState>();
    let user = require_user(&state).await?;
    let devices = ssr::devices(&state, &user).await?;
    let scope = ssr::scope(&user);
    let sms = state
        .store
        .sms()
        .list(scope, None, 10)
        .await
        .map_err(|e| internal("recent SMS", &e))?;
    let calls = state
        .store
        .calls()
        .list(scope, None, 10)
        .await
        .map_err(|e| internal("recent calls", &e))?;
    let unread = state
        .store
        .sms()
        .unread_count(scope)
        .await
        .map_err(|e| internal("SMS non lus", &e))?;
    Ok(AndroidOverviewDto {
        devices: ssr::dtos(&state, &devices).await?,
        recent_sms: ssr::sms_dtos(&state, &sms).await?,
        recent_calls: calls
            .iter()
            .map(dominet_server::android::call_dto)
            .collect(),
        unread: u32::try_from(unread).unwrap_or(u32::MAX),
    })
}

/// The account's devices (all of them for an administrator).
///
/// # Errors
/// Refused (401) without a session; internal error if the database fails.
#[server(name = AndroidDevices, prefix = "/sf", endpoint = "android/devices")]
pub async fn android_devices() -> Result<Vec<AndroidDeviceDto>, ServerFnError> {
    use crate::server_fns::require_user;

    let state = expect_context::<dominet_server::AppState>();
    let user = require_user(&state).await?;
    let devices = ssr::devices(&state, &user).await?;
    ssr::dtos(&state, &devices).await
}

/// Registers a device to the logged-in account; its token is returned **only in this response**.
///
/// # Errors
/// Refused (401) without a session; internal error if the database fails.
#[server(name = AddAndroidDevice, prefix = "/sf", endpoint = "android/devices/add", input = server_fn::codec::Json)]
pub async fn add_android_device(
    input: AndroidDeviceInput,
) -> Result<SaveOutcome<NewAndroidDeviceDto>, ServerFnError> {
    use dominet_domain::android::AndroidDeviceDraft;
    use dominet_domain::repo::AndroidDeviceRepo;

    use crate::server_fns::{field_problems, internal, require_user};

    let state = expect_context::<dominet_server::AppState>();
    let user = require_user(&state).await?;
    let draft = AndroidDeviceDraft {
        name: input.name,
        phone_number: input.phone_number,
    };
    let problems = draft.problems();
    if !problems.is_empty() {
        return Ok(SaveOutcome::Invalid(field_problems(problems)));
    }
    let (device, token) = state
        .store
        .android_devices()
        .create(user.id, &draft)
        .await
        .map_err(|e| internal("creating an Android device", &e))?;
    tracing::info!(
        user_id = user.id.0,
        device = device.id.0,
        "Android device saved"
    );
    let dto = ssr::dtos(&state, &[device]).await?.remove(0);
    Ok(SaveOutcome::Saved(NewAndroidDeviceDto {
        device: dto,
        token,
    }))
}

/// Deletes a device and its data (SMS, calls).
///
/// # Errors
/// Refused (401) without a session; internal error if the database fails.
#[server(name = DeleteAndroidDevice, prefix = "/sf", endpoint = "android/devices/delete")]
pub async fn delete_android_device(id: i64) -> Result<DeleteOutcome, ServerFnError> {
    use dominet_domain::repo::AndroidDeviceRepo;

    use crate::server_fns::{internal, require_user};

    let state = expect_context::<dominet_server::AppState>();
    let user = require_user(&state).await?;
    let Some(device) = ssr::device(&state, &user, id).await? else {
        return Ok(DeleteOutcome::NotFound);
    };
    match state.store.android_devices().delete(device.id).await {
        Ok(()) => {
            tracing::info!(user_id = user.id.0, device = id, "Android device deleted");
            Ok(DeleteOutcome::Deleted)
        }
        Err(dominet_domain::RepoError::NotFound) => Ok(DeleteOutcome::NotFound),
        Err(e) => Err(internal("deleting an Android device", &e)),
    }
}

/// New token of a device (the old one stops working); `None` if the device does not exist or is not the account's.
///
/// # Errors
/// Refused (401) without a session; internal error if the database fails.
#[server(name = RegenerateAndroidToken, prefix = "/sf", endpoint = "android/devices/token")]
pub async fn regenerate_android_token(id: i64) -> Result<Option<String>, ServerFnError> {
    use dominet_domain::repo::AndroidDeviceRepo;

    use crate::server_fns::{internal, require_user};

    let state = expect_context::<dominet_server::AppState>();
    let user = require_user(&state).await?;
    let Some(device) = ssr::device(&state, &user, id).await? else {
        return Ok(None);
    };
    let token = state
        .store
        .android_devices()
        .regenerate_token(device.id)
        .await
        .map_err(|e| internal("regenerating the token", &e))?;
    tracing::info!(
        user_id = user.id.0,
        device = id,
        "Android token regenerated"
    );
    Ok(Some(token))
}

/// The last 100 SMS of a device (the first device if `device` is absent); received SMS are marked read on display,
/// like the reference. `None`: unknown device or not the account's.
///
/// # Errors
/// Refused (401) without a session; internal error if the database fails.
#[server(name = AndroidSmsPage, prefix = "/sf", endpoint = "android/sms")]
pub async fn android_sms_page(
    device: Option<i64>,
) -> Result<Option<AndroidSmsPageDto>, ServerFnError> {
    use dominet_domain::android::AndroidDeviceId;
    use dominet_domain::repo::SmsRepo;

    use crate::server_fns::{internal, require_user};

    let state = expect_context::<dominet_server::AppState>();
    let user = require_user(&state).await?;
    let devices = ssr::devices(&state, &user).await?;
    let current = match device {
        Some(id) => match ssr::device(&state, &user, id).await? {
            Some(d) => Some(d),
            None => return Ok(None),
        },
        None => devices.first().cloned(),
    };
    let mut sms = vec![];
    if let Some(d) = &current {
        state
            .store
            .sms()
            .mark_read(AndroidDeviceId(d.id.0))
            .await
            .map_err(|e| internal("SMS lus", &e))?;
        sms = state
            .store
            .sms()
            .list(ssr::scope(&user), Some(d.id), 100)
            .await
            .map_err(|e| internal("SMS", &e))?;
    }
    let current_dto = match &current {
        Some(d) => ssr::dtos(&state, std::slice::from_ref(d))
            .await?
            .into_iter()
            .next(),
        None => None,
    };
    Ok(Some(AndroidSmsPageDto {
        device: current_dto,
        devices: ssr::dtos(&state, &devices).await?,
        sms: ssr::sms_dtos(&state, &sms).await?,
    }))
}

/// The last 100 calls of a device. `None`: unknown device or not the account's.
///
/// # Errors
/// Refused (401) without a session; internal error if the database fails.
#[server(name = AndroidCallsPage, prefix = "/sf", endpoint = "android/calls")]
pub async fn android_calls_page(
    device: Option<i64>,
) -> Result<Option<AndroidCallsPageDto>, ServerFnError> {
    use dominet_domain::repo::CallRepo;

    use crate::server_fns::{internal, require_user};

    let state = expect_context::<dominet_server::AppState>();
    let user = require_user(&state).await?;
    let devices = ssr::devices(&state, &user).await?;
    let current = match device {
        Some(id) => match ssr::device(&state, &user, id).await? {
            Some(d) => Some(d),
            None => return Ok(None),
        },
        None => devices.first().cloned(),
    };
    let mut calls = vec![];
    if let Some(d) = &current {
        calls = state
            .store
            .calls()
            .list(ssr::scope(&user), Some(d.id), 100)
            .await
            .map_err(|e| internal("calls", &e))?;
    }
    let current_dto = match &current {
        Some(d) => ssr::dtos(&state, std::slice::from_ref(d))
            .await?
            .into_iter()
            .next(),
        None => None,
    };
    Ok(Some(AndroidCallsPageDto {
        device: current_dto,
        devices: ssr::dtos(&state, &devices).await?,
        calls: calls
            .iter()
            .map(dominet_server::android::call_dto)
            .collect(),
    }))
}

/// Sends an SMS through a device: records the outgoing SMS then hands it to the device, which must be connected.
///
/// # Errors
/// Refused (401) without a session; internal error if the database fails.
#[server(name = SendAndroidSms, prefix = "/sf", endpoint = "android/sms/send")]
pub async fn send_android_sms(
    device: i64,
    to: String,
    body: String,
) -> Result<SendSmsOutcome, ServerFnError> {
    use dominet_domain::android::{NewSms, OutboxOrigin, SmsDirection, SmsDraft};
    use dominet_domain::repo::SmsRepo;
    use dominet_proto::AndroidEvent;
    use dominet_server::android::SendError;

    use crate::server_fns::{field_problems, internal, require_user};

    let state = expect_context::<dominet_server::AppState>();
    let user = require_user(&state).await?;
    let Some(found) = ssr::device(&state, &user, device).await? else {
        return Ok(SendSmsOutcome::NotFound);
    };
    let draft = SmsDraft {
        to: to.trim().to_string(),
        body,
    };
    let problems = draft.problems();
    if !problems.is_empty() {
        return Ok(SendSmsOutcome::Invalid(field_problems(problems)));
    }
    if !state.android.is_connected(found.id) {
        return Ok(SendSmsOutcome::Offline);
    }
    let saved = state
        .store
        .sms()
        .insert(&NewSms {
            device: found.id,
            direction: SmsDirection::Outgoing,
            phone_number: draft.to.clone(),
            contact_name: String::new(),
            body: draft.body.clone(),
            is_read: true,
            sent_at: chrono::Utc::now(),
        })
        .await
        .map_err(|e| internal("outgoing SMS", &e))?;
    match state
        .android
        .send_sms(
            &state,
            found.id,
            &draft.to,
            &draft.body,
            OutboxOrigin::Browser(saved.id),
        )
        .await
    {
        Ok(item) => {
            let dto = dominet_server::android::sms_dto(&saved, Some(&item));
            dominet_server::android::publish(
                &state.hub,
                found.id,
                AndroidEvent::SmsQueued { sms: dto.clone() },
            );
            tracing::info!(
                user_id = user.id.0,
                device = device,
                "SMS handed to the device"
            );
            Ok(SendSmsOutcome::Queued(dto))
        }
        Err(SendError::Offline) => Ok(SendSmsOutcome::Offline),
        Err(SendError::Repo(e)) => Err(internal("send queue", &e)),
    }
}

// ------------------------------------------------------------------ views

/// Follows the given devices live: the events increment `Realtime::android`, which reloads the pages.
fn follow(ids: Signal<Vec<i64>>) {
    let rt = expect_context::<Realtime>();
    let held = StoredValue::new(Vec::<i64>::new());
    Effect::new(move |_| {
        let now = ids.get();
        let before = held.get_value();
        for id in before.iter().filter(|i| !now.contains(i)) {
            rt.unsubscribe(&Topic::Android(*id));
        }
        for id in now.iter().filter(|i| !before.contains(i)) {
            rt.subscribe(Topic::Android(*id));
        }
        held.set_value(now);
    });
    on_cleanup(move || {
        for id in held.get_value() {
            rt.unsubscribe(&Topic::Android(id));
        }
    });
}

/// RFC 3339 date in server local time -> `2026-11-03 09:35:00` (the offset is removed).
pub(crate) fn shown(time: &str) -> String {
    time.get(..19).unwrap_or(time).replace('T', " ")
}

fn mmss(seconds: i32) -> String {
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

fn call_direction(direction: &str) -> String {
    match direction {
        "outgoing" => tr!("android-call-outgoing"),
        "missed" => tr!("android-call-missed"),
        _ => tr!("android-call-incoming"),
    }
}

fn delivery_label(sms: &SmsDto) -> String {
    match sms.delivery.as_str() {
        "pending" => tr!("android-sms-pending"),
        "sent" => tr!("android-sms-sent"),
        "failed" => {
            if sms.error.is_empty() {
                tr!("android-sms-failed")
            } else {
                format!("{} ({})", tr!("android-sms-failed"), sms.error)
            }
        }
        _ => String::new(),
    }
}

#[component]
fn Presence(online: bool) -> impl IntoView {
    view! {
        <span class="vc-badge" class:vc-badge--success=online data-online=online.to_string()>
            {if online { tr!("android-online") } else { tr!("android-offline") }}
        </span>
    }
}

/// Tabs for choosing a page's device (`base` = `/android/sms` or `/android/calls`).
#[component]
fn DeviceTabs(devices: Vec<AndroidDeviceDto>, current: i64, base: &'static str) -> impl IntoView {
    view! {
        <nav class="vc-tabs" id="android-device-tabs">
            {devices
                .into_iter()
                .map(|d| {
                    let id = d.id;
                    view! {
                        <A
                            href=format!("{base}?device={id}")
                            attr:class="vc-btn vc-btn--ghost vc-btn--sm"
                            attr:id=format!("android-tab-{id}")
                            attr:aria-current=(id == current).then_some("page")
                        >
                            {d.name}
                        </A>
                    }
                })
                .collect_view()}
        </nav>
    }
}

fn device_param() -> impl Fn() -> Option<i64> {
    let query = use_query_map();
    move || query.with(|q| q.get("device").and_then(|d| d.parse::<i64>().ok()))
}

#[component]
fn NoDevice() -> impl IntoView {
    view! {
        <p class="vc-muted" id="android-no-device">
            {move_tr!("android-no-device")} " " <A href="/android/devices">{move_tr!("android-add-one")}</A>
        </p>
    }
}

// ------------------------------------------------------------------ dashboard

#[component]
pub fn AndroidDashboardPage() -> impl IntoView {
    view! {
        <RequireAuth>
            <main class="vc-container">
                <AndroidDashboard/>
            </main>
        </RequireAuth>
    }
}

#[component]
fn AndroidDashboard() -> impl IntoView {
    let rt = expect_context::<Realtime>();
    let data = Resource::new(
        move || rt.android.get(),
        |_| async { android_overview().await.map_err(|_| ()) },
    );
    follow(Signal::derive(move || match data.get() {
        Some(Ok(d)) => d.devices.iter().map(|x| x.id).collect(),
        _ => vec![],
    }));
    view! {
        <div class="vc-page-header">
            <h2>{move_tr!("android-title")}</h2>
            <div class="vc-header-actions">
                <A href="/android/devices" attr:class="vc-btn vc-btn--ghost">{move_tr!("android-devices")}</A>
                <A href="/android/sms" attr:class="vc-btn vc-btn--ghost" attr:id="android-go-sms">{move_tr!("android-sms")}</A>
                <A href="/android/calls" attr:class="vc-btn vc-btn--ghost">{move_tr!("android-calls")}</A>
            </div>
        </div>
        <Transition fallback=|| ()>
            {move || match data.get() {
                None => ().into_any(),
                Some(Err(())) => {
                    view! { <div class="vc-alert vc-alert--error" role="alert">{move_tr!("crud-load-error")}</div> }.into_any()
                }
                Some(Ok(d)) => {
                    let names: std::collections::HashMap<i64, String> =
                        d.devices.iter().map(|x| (x.id, x.name.clone())).collect();
                    let name_of = {
                        let names = names.clone();
                        move |id: i64| names.get(&id).cloned().unwrap_or_default()
                    };
                    let name_of_calls = name_of.clone();
                    view! {
                        <p id="android-unread" data-unread=d.unread.to_string()>
                            {move || tr!("android-unread", {"count" => d.unread})}
                        </p>
                        <div class="vc-card">
                            <h3>{move_tr!("android-devices")}</h3>
                            {if d.devices.is_empty() {
                                view! { <NoDevice/> }.into_any()
                            } else {
                                view! {
                                    <table class="vc-table" id="android-devices-table">
                                        <tbody>
                                            {d
                                                .devices
                                                .into_iter()
                                                .map(|x| {
                                                    view! {
                                                        <tr data-id=x.id>
                                                            <td>{x.name}</td>
                                                            <td>{x.owner}</td>
                                                            <td><Presence online=x.is_online/></td>
                                                            <td class="vc-muted">{shown(&x.last_seen)}</td>
                                                        </tr>
                                                    }
                                                })
                                                .collect_view()}
                                        </tbody>
                                    </table>
                                }
                                    .into_any()
                            }}
                        </div>
                        <div class="vc-card">
                            <h3>{move_tr!("android-recent-sms")}</h3>
                            <ul class="vc-list" id="android-recent-sms">
                                {d
                                    .recent_sms
                                    .into_iter()
                                    .map(|s| {
                                        view! {
                                            <li data-id=s.id data-direction=s.direction.clone()>
                                                <strong>{s.phone_number.clone()}</strong>
                                                " "
                                                <span class="vc-muted">{name_of(s.device_id)}</span>
                                                <div>{s.body.clone()}</div>
                                            </li>
                                        }
                                    })
                                    .collect_view()}
                            </ul>
                        </div>
                        <div class="vc-card">
                            <h3>{move_tr!("android-recent-calls")}</h3>
                            <ul class="vc-list" id="android-recent-calls">
                                {d
                                    .recent_calls
                                    .into_iter()
                                    .map(|c| {
                                        view! {
                                            <li data-id=c.id data-direction=c.direction.clone()>
                                                <strong>{c.phone_number.clone()}</strong>
                                                " "
                                                <span class="vc-muted">
                                                    {call_direction(&c.direction)} " " {name_of_calls(c.device_id)}
                                                </span>
                                            </li>
                                        }
                                    })
                                    .collect_view()}
                            </ul>
                        </div>
                    }
                        .into_any()
                }
            }}
        </Transition>
    }
}

// ------------------------------------------------------------------ devices

#[component]
pub fn AndroidDevicesPage() -> impl IntoView {
    view! {
        <RequireAuth>
            <main class="vc-container">
                <AndroidDevices/>
            </main>
        </RequireAuth>
    }
}

#[component]
fn AndroidDevices() -> impl IntoView {
    let rt = expect_context::<Realtime>();
    let ready = use_hydrated();
    let version = RwSignal::new(0u32);
    let list = Resource::new(
        move || (version.get(), rt.android.get()),
        |_| async { android_devices().await.map_err(|_| ()) },
    );
    follow(Signal::derive(move || match list.get() {
        Some(Ok(d)) => d.iter().map(|x| x.id).collect(),
        _ => vec![],
    }));
    let name = NodeRef::<leptos::html::Input>::new();
    let phone = NodeRef::<leptos::html::Input>::new();
    let problems = RwSignal::new(Vec::<FieldProblem>::new());
    // The token is only shown here, after creation or regeneration, until it is hidden.
    let token = RwSignal::new(None::<(String, String)>);
    let confirm = RwSignal::new(None::<i64>);
    let add = Action::new(|input: &AndroidDeviceInput| {
        let input = input.clone();
        async move { add_android_device(input).await }
    });
    let regenerate = Action::new(|id: &i64| {
        let id = *id;
        async move { (id, regenerate_android_token(id).await) }
    });
    let delete = Action::new(|id: &i64| {
        let id = *id;
        async move { delete_android_device(id).await }
    });
    Effect::new(move |_| {
        if let Some(Ok(outcome)) = add.value().get() {
            match outcome {
                SaveOutcome::Saved(NewAndroidDeviceDto { device, token: t }) => {
                    problems.set(vec![]);
                    clear_input(name);
                    clear_input(phone);
                    token.set(Some((device.name, t)));
                    version.update(|v| *v += 1);
                }
                SaveOutcome::Invalid(p) => problems.set(p),
                SaveOutcome::NotFound | SaveOutcome::Blocked(_) => {}
            }
        }
    });
    Effect::new(move |_| {
        if let Some((id, Ok(Some(t)))) = regenerate.value().get() {
            let label = list
                .get_untracked()
                .and_then(Result::ok)
                .and_then(|d| d.into_iter().find(|x| x.id == id))
                .map(|x| x.name)
                .unwrap_or_default();
            token.set(Some((label, t)));
        }
    });
    Effect::new(move |_| {
        if delete.value().get().is_some() {
            confirm.set(None);
            version.update(|v| *v += 1);
        }
    });
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        add.dispatch(AndroidDeviceInput {
            name: input_value(name),
            phone_number: input_value(phone),
        });
    };
    view! {
        <div class="vc-page-header">
            <h2>{move_tr!("android-devices")}</h2>
            <div class="vc-header-actions">
                <A href="/android" attr:class="vc-btn vc-btn--ghost">{move_tr!("android-title")}</A>
            </div>
        </div>
        <Show when=move || token.with(Option::is_some)>
            <div class="vc-alert vc-alert--warning" id="android-token" role="status">
                <p>{move || tr!("android-token-once", {"name" => token.get().map(|t| t.0).unwrap_or_default()})}</p>
                <p>
                    <code id="android-token-value">{move || token.get().map(|t| t.1).unwrap_or_default()}</code>
                </p>
                <p class="vc-muted">{move_tr!("android-token-hint")}</p>
                <button type="button" id="android-token-hide" class="vc-btn vc-btn--ghost vc-btn--sm" on:click=move |_| token.set(None)>
                    {move_tr!("android-token-hide")}
                </button>
            </div>
        </Show>
        <div class="vc-card">
            <h3>{move_tr!("android-add")}</h3>
            <form on:submit=submit id="android-add-form">
                <div class="vc-form-group">
                    <label for="android-name">{move_tr!("android-name")}</label>
                    <input id="android-name" type="text" node_ref=name maxlength="100" placeholder=move || tr!("android-name-placeholder")/>
                    {field_error("name", problems)}
                </div>
                <div class="vc-form-group">
                    <label for="android-phone">{move_tr!("android-phone")}</label>
                    <input id="android-phone" type="text" node_ref=phone maxlength="20" placeholder="+33612345678"/>
                    {field_error("phone_number", problems)}
                </div>
                <button type="submit" id="android-add-submit" class="vc-btn vc-btn--primary" disabled=move || !ready.get() || add.pending().get()>
                    {move_tr!("android-add-button")}
                </button>
            </form>
        </div>
        <Transition fallback=|| ()>
            {move || match list.get() {
                None => ().into_any(),
                Some(Err(())) => {
                    view! { <div class="vc-alert vc-alert--error" role="alert">{move_tr!("crud-load-error")}</div> }.into_any()
                }
                Some(Ok(devices)) if devices.is_empty() => {
                    view! { <p class="vc-muted" id="android-empty">{move_tr!("android-no-device")}</p> }.into_any()
                }
                Some(Ok(devices)) => {
                    view! {
                        <div class="vc-card">
                            <table class="vc-table" id="android-devices-table">
                                <thead>
                                    <tr>
                                        <th>{move_tr!("android-name")}</th>
                                        <th>{move_tr!("android-phone")}</th>
                                        <th>{move_tr!("android-owner")}</th>
                                        <th>{move_tr!("android-state")}</th>
                                        <th>{move_tr!("android-last-seen")}</th>
                                        <th></th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {devices
                                        .into_iter()
                                        .map(|d| {
                                            let id = d.id;
                                            view! {
                                                <tr data-id=id data-online=d.is_online.to_string()>
                                                    <td>{d.name.clone()}</td>
                                                    <td>{d.phone_number.clone()}</td>
                                                    <td>{d.owner.clone()}</td>
                                                    <td><Presence online=d.is_online/></td>
                                                    <td class="vc-muted">{shown(&d.last_seen)}</td>
                                                    <td>
                                                        <button
                                                            type="button"
                                                            class="vc-btn vc-btn--ghost vc-btn--sm android-regenerate"
                                                            disabled=move || !ready.get() || regenerate.pending().get()
                                                            on:click=move |_| {
                                                                regenerate.dispatch(id);
                                                            }
                                                        >
                                                            {move_tr!("android-regenerate")}
                                                        </button>
                                                        <Show
                                                            when=move || confirm.get() == Some(id)
                                                            fallback=move || {
                                                                view! {
                                                                    <button
                                                                        type="button"
                                                                        class="vc-btn vc-btn--ghost vc-btn--sm android-delete"
                                                                        disabled=move || !ready.get()
                                                                        on:click=move |_| confirm.set(Some(id))
                                                                    >
                                                                        {move_tr!("crud-delete")}
                                                                    </button>
                                                                }
                                                            }
                                                        >
                                                            <button
                                                                type="button"
                                                                class="vc-btn vc-btn--danger vc-btn--sm android-delete-confirm"
                                                                disabled=move || delete.pending().get()
                                                                on:click=move |_| {
                                                                    delete.dispatch(id);
                                                                }
                                                            >
                                                                {move_tr!("android-delete-confirm")}
                                                            </button>
                                                            <button
                                                                type="button"
                                                                class="vc-btn vc-btn--ghost vc-btn--sm"
                                                                on:click=move |_| confirm.set(None)
                                                            >
                                                                {move_tr!("crud-cancel")}
                                                            </button>
                                                        </Show>
                                                    </td>
                                                </tr>
                                            }
                                        })
                                        .collect_view()}
                                </tbody>
                            </table>
                        </div>
                    }
                        .into_any()
                }
            }}
        </Transition>
    }
}

// ------------------------------------------------------------------ SMS

#[component]
pub fn AndroidSmsPage() -> impl IntoView {
    view! {
        <RequireAuth>
            <main class="vc-container">
                <AndroidSms/>
            </main>
        </RequireAuth>
    }
}

#[component]
fn AndroidSms() -> impl IntoView {
    let rt = expect_context::<Realtime>();
    let ready = use_hydrated();
    let device = device_param();
    let version = RwSignal::new(0u32);
    let data = Resource::new(
        move || (device(), version.get(), rt.android.get()),
        |(d, _, _)| async move { android_sms_page(d).await.map_err(|_| ()) },
    );
    follow(Signal::derive(move || match data.get() {
        Some(Ok(Some(p))) => p.device.map(|d| vec![d.id]).unwrap_or_default(),
        _ => vec![],
    }));
    let to = NodeRef::<leptos::html::Input>::new();
    let body = NodeRef::<leptos::html::Textarea>::new();
    let problems = RwSignal::new(Vec::<FieldProblem>::new());
    let notice = RwSignal::new(None::<&'static str>);
    let send = Action::new(|(device, to, body): &(i64, String, String)| {
        let (device, to, body) = (*device, to.clone(), body.clone());
        async move { send_android_sms(device, to, body).await }
    });
    Effect::new(move |_| {
        if let Some(Ok(outcome)) = send.value().get() {
            match outcome {
                SendSmsOutcome::Queued(_) => {
                    problems.set(vec![]);
                    notice.set(Some("queued"));
                    clear_input(to);
                    if let Some(b) = body.get_untracked() {
                        b.set_value("");
                    }
                    version.update(|v| *v += 1);
                }
                SendSmsOutcome::Invalid(p) => {
                    notice.set(None);
                    problems.set(p);
                }
                SendSmsOutcome::Offline => {
                    problems.set(vec![]);
                    notice.set(Some("offline"));
                }
                SendSmsOutcome::NotFound => notice.set(Some("not-found")),
            }
        }
    });
    view! {
        <div class="vc-page-header">
            <h2>{move_tr!("android-sms")}</h2>
            <div class="vc-header-actions">
                <A href="/android" attr:class="vc-btn vc-btn--ghost">{move_tr!("android-title")}</A>
            </div>
        </div>
        <Transition fallback=|| ()>
            {move || match data.get() {
                None => ().into_any(),
                Some(Err(())) => {
                    view! { <div class="vc-alert vc-alert--error" role="alert">{move_tr!("crud-load-error")}</div> }.into_any()
                }
                Some(Ok(None)) => {
                    view! { <p class="vc-alert vc-alert--error" id="android-not-found">{move_tr!("android-not-found")}</p> }
                        .into_any()
                }
                Some(Ok(Some(page))) => {
                    let Some(current) = page.device.clone() else {
                        return view! { <NoDevice/> }.into_any();
                    };
                    let id = current.id;
                    view! {
                        <DeviceTabs devices=page.devices.clone() current=id base="/android/sms"/>
                        <p id="android-presence" data-online=current.is_online.to_string()>
                            <Presence online=current.is_online/>
                        </p>
                        <div class="vc-card">
                            <form
                                id="android-send-form"
                                on:submit=move |ev: leptos::ev::SubmitEvent| {
                                    ev.prevent_default();
                                    let text = body.get_untracked().map(|b| b.value()).unwrap_or_default();
                                    send.dispatch((id, input_value(to), text));
                                }
                            >
                                <div class="vc-form-group">
                                    <label for="android-to">{move_tr!("android-to")}</label>
                                    <input id="android-to" type="text" node_ref=to maxlength="30" placeholder="+33612345678"/>
                                    {field_error("to", problems)}
                                </div>
                                <div class="vc-form-group">
                                    <label for="android-body">{move_tr!("android-body")}</label>
                                    <textarea id="android-body" node_ref=body rows="4" maxlength="1600"></textarea>
                                    {field_error("body", problems)}
                                </div>
                                <button
                                    type="submit"
                                    id="android-send"
                                    class="vc-btn vc-btn--primary"
                                    disabled=move || !ready.get() || send.pending().get()
                                >
                                    {move_tr!("android-send")}
                                </button>
                                <Show when=move || notice.get().is_some()>
                                    <span class="vc-muted" id="android-notice" data-notice=move || notice.get().unwrap_or_default()>
                                        {move || match notice.get() {
                                            Some("queued") => tr!("android-notice-queued"),
                                            Some("offline") => tr!("android-notice-offline"),
                                            _ => tr!("android-not-found"),
                                        }}
                                    </span>
                                </Show>
                            </form>
                        </div>
                        {if page.sms.is_empty() {
                            view! { <p class="vc-muted" id="android-sms-empty">{move_tr!("android-sms-empty")}</p> }.into_any()
                        } else {
                            view! {
                                <div class="vc-card">
                                    <ul class="vc-list" id="android-sms-list">
                                        {page
                                            .sms
                                            .into_iter()
                                            .map(|s| {
                                                let label = delivery_label(&s);
                                                view! {
                                                    <li data-id=s.id data-direction=s.direction.clone() data-delivery=s.delivery.clone()>
                                                        <strong>{s.phone_number.clone()}</strong>
                                                        " "
                                                        <span class="vc-muted">{s.contact_name.clone()} " " {shown(&s.sent_at)}</span>
                                                        <div class="android-sms-body">{s.body.clone()}</div>
                                                        <div class="vc-muted android-sms-delivery">{label}</div>
                                                    </li>
                                                }
                                            })
                                            .collect_view()}
                                    </ul>
                                </div>
                            }
                                .into_any()
                        }}
                    }
                        .into_any()
                }
            }}
        </Transition>
    }
}

// ------------------------------------------------------------------ call log

#[component]
pub fn AndroidCallsPage() -> impl IntoView {
    view! {
        <RequireAuth>
            <main class="vc-container">
                <AndroidCalls/>
            </main>
        </RequireAuth>
    }
}

#[component]
fn AndroidCalls() -> impl IntoView {
    let rt = expect_context::<Realtime>();
    let device = device_param();
    let data = Resource::new(
        move || (device(), rt.android.get()),
        |(d, _)| async move { android_calls_page(d).await.map_err(|_| ()) },
    );
    follow(Signal::derive(move || match data.get() {
        Some(Ok(Some(p))) => p.device.map(|d| vec![d.id]).unwrap_or_default(),
        _ => vec![],
    }));
    view! {
        <div class="vc-page-header">
            <h2>{move_tr!("android-calls")}</h2>
            <div class="vc-header-actions">
                <A href="/android" attr:class="vc-btn vc-btn--ghost">{move_tr!("android-title")}</A>
            </div>
        </div>
        <Transition fallback=|| ()>
            {move || match data.get() {
                None => ().into_any(),
                Some(Err(())) => {
                    view! { <div class="vc-alert vc-alert--error" role="alert">{move_tr!("crud-load-error")}</div> }.into_any()
                }
                Some(Ok(None)) => {
                    view! { <p class="vc-alert vc-alert--error" id="android-not-found">{move_tr!("android-not-found")}</p> }
                        .into_any()
                }
                Some(Ok(Some(page))) => {
                    let Some(current) = page.device.clone() else {
                        return view! { <NoDevice/> }.into_any();
                    };
                    view! {
                        <DeviceTabs devices=page.devices.clone() current=current.id base="/android/calls"/>
                        {if page.calls.is_empty() {
                            view! { <p class="vc-muted" id="android-calls-empty">{move_tr!("android-calls-empty")}</p> }.into_any()
                        } else {
                            view! {
                                <div class="vc-card">
                                    <table class="vc-table" id="android-calls-table">
                                        <thead>
                                            <tr>
                                                <th>{move_tr!("android-call-type")}</th>
                                                <th>{move_tr!("android-phone")}</th>
                                                <th>{move_tr!("android-call-started")}</th>
                                                <th>{move_tr!("android-call-duration")}</th>
                                            </tr>
                                        </thead>
                                        <tbody>
                                            {page.calls.into_iter().map(|c| call_row(&c)).collect_view()}
                                        </tbody>
                                    </table>
                                </div>
                            }
                                .into_any()
                        }}
                    }
                        .into_any()
                }
            }}
        </Transition>
    }
}

fn call_row(c: &CallDto) -> impl IntoView + use<> {
    let who = if c.contact_name.is_empty() {
        c.phone_number.clone()
    } else {
        format!("{} ({})", c.contact_name, c.phone_number)
    };
    view! {
        <tr data-id=c.id data-direction=c.direction.clone() data-open=c.ended_at.is_none().to_string()>
            <td>{call_direction(&c.direction)}</td>
            <td>{who}</td>
            <td>{shown(&c.started_at)}</td>
            <td>{if c.ended_at.is_some() { mmss(c.duration) } else { tr!("android-call-ongoing") }}</td>
        </tr>
    }
}
