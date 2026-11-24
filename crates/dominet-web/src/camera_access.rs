// SPDX-License-Identifier: AGPL-3.0-or-later
//! "Camera access" page: `/account/cameras` (any signed-in account; ADR 0064).
//!
//! MediaMTX asks DomiNet before every read. The browser has its own short-lived token; an external player (VLC, a phone application)
//! needs the account's user name and a **personal stream credential** created here. The password is shown once, right after its
//! creation; only its hash is stored. A credential is revoked with one click, and the sessions the account has open are closed.

use dominet_proto::{FieldProblem, SaveOutcome};
use leptos::prelude::*;
use leptos_fluent::{move_tr, tr};
use serde::{Deserialize, Serialize};

use crate::browser::{clear_input, input_value};
use crate::crud::field_error;
use crate::guard::RequireAuth;
use crate::layout::use_hydrated;

/// A credential as the list shows it (no secret: the prefix is only the start of the password).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamCredentialDto {
    pub id: i64,
    pub name: String,
    pub prefix: String,
    pub created_at: String,
    pub expires_at: String,
    /// Empty if never used.
    pub last_used_at: String,
    /// `active`, `expired` or `revoked`.
    pub state: String,
}

/// A camera and the address an external player opens (no secret in it).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraAddressDto {
    pub name: String,
    pub address: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraAccessPageDto {
    /// The user name to give the player: the account's.
    pub username: String,
    pub cameras: Vec<CameraAddressDto>,
    pub credentials: Vec<StreamCredentialDto>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamCredentialInput {
    pub name: String,
    pub days: u32,
}

/// Answer to a creation: the password, **only here**.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatedStreamCredentialDto {
    pub username: String,
    pub password: String,
    pub credential: StreamCredentialDto,
}

/// Answer to a revocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamRevocationDto {
    /// `false` if the credential was not active (or not the account's).
    pub revoked: bool,
    /// Open sessions closed; `None` if MediaMTX could not be reached (the credential is refused from the next connection anyway).
    pub closed: Option<u32>,
}

#[cfg(feature = "ssr")]
mod ssr {
    use chrono::{DateTime, Utc};
    use dominet_domain::stream_access::{AccessState, StreamAccess};
    use dominet_server::timefmt;

    use super::StreamCredentialDto;

    /// Port of MediaMTX's RTSP listener as published by the production stack.
    pub const RTSP_PORT: u16 = 8554;

    pub fn dto(a: &StreamAccess, now: DateTime<Utc>) -> StreamCredentialDto {
        StreamCredentialDto {
            id: a.id.0,
            name: a.name.clone(),
            prefix: a.prefix.clone(),
            created_at: timefmt::text(a.created_at),
            expires_at: timefmt::text(a.expires_at),
            last_used_at: a.last_used_at.map(timefmt::text).unwrap_or_default(),
            state: match a.state(now) {
                AccessState::Active => "active",
                AccessState::Expired => "expired",
                AccessState::Revoked => "revoked",
            }
            .to_string(),
        }
    }

    /// Host name of a `Host` header, without its port (IPv6 literals keep their brackets).
    pub fn bare_host(host: &str) -> &str {
        if host.starts_with('[') {
            host.split_inclusive(']').next().unwrap_or(host)
        } else {
            host.split(':').next().unwrap_or(host)
        }
    }
}

/// The account's user name, the cameras with their RTSP addresses and the account's credentials.
///
/// # Errors
/// Refused (401) without a session; internal error if the database fails.
#[server(name = CameraAccessPage, prefix = "/sf", endpoint = "camera-access/page")]
pub async fn camera_access_page() -> Result<CameraAccessPageDto, ServerFnError> {
    use chrono::Utc;
    use dominet_domain::repo::StreamAccessRepo;

    use crate::server_fns::{internal, require_user};

    let state = expect_context::<dominet_server::AppState>();
    let user = require_user(&state).await?;
    let host = crate::camera_fns::request_host().await;
    let host = host.as_deref().map_or("<host>", ssr::bare_host).to_string();
    let cameras = state
        .cameras
        .list()
        .await
        .map_err(|e| internal("reading the cameras", &e))?
        .into_iter()
        .filter(|c| c.config.active)
        .map(|c| CameraAddressDto {
            name: c.config.name.clone(),
            address: format!("rtsp://{host}:{}/{}", ssr::RTSP_PORT, c.uuid),
        })
        .collect();
    let now = Utc::now();
    let credentials = state
        .store
        .stream_access()
        .list_credentials(user.id)
        .await
        .map_err(|e| internal("reading the stream credentials", &e))?;
    Ok(CameraAccessPageDto {
        username: user.username,
        cameras,
        credentials: credentials.iter().map(|a| ssr::dto(a, now)).collect(),
    })
}

/// Creates a credential for the account. The password is in this answer and nowhere else: it cannot be read back.
///
/// # Errors
/// Refused (401) without a session; internal error if the database fails.
#[server(name = CreateStreamCredential, prefix = "/sf", endpoint = "camera-access/create", input = server_fn::codec::Json)]
pub async fn create_stream_credential(
    input: StreamCredentialInput,
) -> Result<SaveOutcome<CreatedStreamCredentialDto>, ServerFnError> {
    use chrono::Utc;
    use dominet_domain::stream_access::NewStreamCredential;

    use crate::server_fns::{field_problems, internal, require_user};

    let state = expect_context::<dominet_server::AppState>();
    let user = require_user(&state).await?;
    let new = match NewStreamCredential::new(&input.name, input.days) {
        Ok(new) => new,
        Err(problems) => return Ok(SaveOutcome::Invalid(field_problems(problems))),
    };
    let issued = dominet_server::stream_access::issue_credential(&state, user.id, &new)
        .await
        .map_err(|e| internal("creating a stream credential", &e))?;
    // The password is never logged: only who created which credential.
    tracing::info!(
        user_id = user.id.0,
        credential_id = issued.access.id.0,
        "stream credential created"
    );
    Ok(SaveOutcome::Saved(CreatedStreamCredentialDto {
        username: user.username,
        credential: ssr::dto(&issued.access, Utc::now()),
        password: issued.password,
    }))
}

/// Revokes one of the account's credentials: refused from the next connection, and the account's open sessions are closed.
///
/// # Errors
/// Refused (401) without a session; internal error if the database fails.
#[server(name = RevokeStreamCredential, prefix = "/sf", endpoint = "camera-access/revoke")]
pub async fn revoke_stream_credential(id: i64) -> Result<StreamRevocationDto, ServerFnError> {
    use dominet_domain::stream_access::StreamAccessId;

    use crate::server_fns::{internal, require_user};

    let state = expect_context::<dominet_server::AppState>();
    let user = require_user(&state).await?;
    let outcome = dominet_server::stream_access::revoke_credential(
        &state,
        user.id,
        &user.username,
        StreamAccessId(id),
    )
    .await
    .map_err(|e| internal("revoking a stream credential", &e))?;
    if outcome.revoked {
        tracing::info!(
            user_id = user.id.0,
            credential_id = id,
            "stream credential revoked"
        );
    }
    Ok(StreamRevocationDto {
        revoked: outcome.revoked,
        closed: outcome.closed,
    })
}

fn state_text(state: &str) -> String {
    match state {
        "active" => tr!("camaccess-state-active"),
        "expired" => tr!("camaccess-state-expired"),
        _ => tr!("camaccess-state-revoked"),
    }
}

#[component]
pub fn CameraAccessPage() -> impl IntoView {
    view! {
        <RequireAuth>
            <main class="vc-container">
                <CameraAccess/>
            </main>
        </RequireAuth>
    }
}

#[component]
fn CameraAccess() -> impl IntoView {
    let version = RwSignal::new(0u32);
    // The password of a credential just created: shown once, until hidden or until the page is left.
    let created = RwSignal::new(None::<CreatedStreamCredentialDto>);
    // What the last revocation did, to tell the person.
    let revoked = RwSignal::new(None::<StreamRevocationDto>);
    let data = Resource::new(
        move || version.get(),
        |_| async { camera_access_page().await.map_err(|_| ()) },
    );
    view! {
        <div class="vc-page-header">
            <h2>{move_tr!("camaccess-title")}</h2>
        </div>
        <p class="vc-muted">{move_tr!("camaccess-intro")}</p>
        <Transition fallback=|| ()>
            {move || match data.get() {
                None => ().into_any(),
                Some(Err(())) => view! { <div class="vc-alert vc-alert--error" role="alert">{move_tr!("crud-load-error")}</div> }.into_any(),
                Some(Ok(page)) => view! {
                    <CredentialForm version=version created=created revoked=revoked/>
                    <CreatedCredential created=created/>
                    <RevokedNotice revoked=revoked/>
                    <CredentialTable credentials=page.credentials version=version revoked=revoked/>
                    <Addresses username=page.username cameras=page.cameras/>
                }.into_any(),
            }}
        </Transition>
    }
}

#[component]
fn CredentialForm(
    version: RwSignal<u32>,
    created: RwSignal<Option<CreatedStreamCredentialDto>>,
    revoked: RwSignal<Option<StreamRevocationDto>>,
) -> impl IntoView {
    let ready = use_hydrated();
    let name = NodeRef::<leptos::html::Input>::new();
    let days = RwSignal::new(90u32);
    let problems = RwSignal::new(Vec::<FieldProblem>::new());
    let create = Action::new(|input: &StreamCredentialInput| {
        let input = input.clone();
        async move { create_stream_credential(input).await }
    });
    Effect::new(move |_| {
        if let Some(Ok(outcome)) = create.value().get() {
            match outcome {
                SaveOutcome::Saved(c) => {
                    problems.set(vec![]);
                    clear_input(name);
                    revoked.set(None);
                    created.set(Some(c));
                    version.update(|v| *v += 1);
                }
                SaveOutcome::Invalid(p) => problems.set(p),
                SaveOutcome::NotFound | SaveOutcome::Blocked(_) => {}
            }
        }
    });
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        create.dispatch(StreamCredentialInput {
            name: input_value(name),
            days: days.get_untracked(),
        });
    };
    view! {
        <div class="vc-card">
            <h3>{move_tr!("camaccess-create")}</h3>
            <form on:submit=submit id="camaccess-create-form">
                <div class="vc-form-group">
                    <label for="camaccess-name">{move_tr!("camaccess-name")}</label>
                    <input id="camaccess-name" type="text" node_ref=name maxlength="64" placeholder=move || tr!("camaccess-name-placeholder")/>
                    {field_error("name", problems)}
                </div>
                <div class="vc-form-group">
                    <label for="camaccess-days">{move_tr!("camaccess-days")}</label>
                    <select id="camaccess-days" on:change=move |ev| days.set(event_target_value(&ev).parse().unwrap_or(90))>
                        <option value="30" selected=move || days.get() == 30>"30"</option>
                        <option value="90" selected=move || days.get() == 90>"90"</option>
                        <option value="180" selected=move || days.get() == 180>"180"</option>
                        <option value="365" selected=move || days.get() == 365>"365"</option>
                    </select>
                    {field_error("lifetime_days", problems)}
                </div>
                <button type="submit" id="camaccess-create-submit" class="vc-btn vc-btn--primary" disabled=move || !ready.get() || create.pending().get()>
                    {move_tr!("camaccess-create-button")}
                </button>
            </form>
        </div>
    }
}

/// The password just created, shown once with the user name to type next to it.
#[component]
fn CreatedCredential(created: RwSignal<Option<CreatedStreamCredentialDto>>) -> impl IntoView {
    view! {
        <Show when=move || created.with(Option::is_some)>
            <div class="vc-alert vc-alert--warning" id="camaccess-secret" role="status">
                <p>{move || tr!("camaccess-once", {"name" => created.get().map(|c| c.credential.name).unwrap_or_default()})}</p>
                <p>{move_tr!("camaccess-user")}" : "<code id="camaccess-secret-user">{move || created.get().map(|c| c.username).unwrap_or_default()}</code></p>
                <p>{move_tr!("camaccess-password")}" : "<code id="camaccess-secret-password">{move || created.get().map(|c| c.password).unwrap_or_default()}</code></p>
                <button type="button" id="camaccess-hide" class="vc-btn vc-btn--ghost vc-btn--sm" on:click=move |_| created.set(None)>
                    {move_tr!("camaccess-hide")}
                </button>
            </div>
        </Show>
    }
}

#[component]
fn RevokedNotice(revoked: RwSignal<Option<StreamRevocationDto>>) -> impl IntoView {
    view! {
        <Show when=move || revoked.with(Option::is_some)>
            <div class="vc-alert vc-alert--success" id="camaccess-revoked" role="status">
                {move || match revoked.get() {
                    Some(StreamRevocationDto { revoked: false, .. }) => tr!("camaccess-revoked-none"),
                    Some(StreamRevocationDto { closed: Some(n), .. }) => tr!("camaccess-revoked-closed", {"n" => n.to_string()}),
                    _ => tr!("camaccess-revoked-unreachable"),
                }}
            </div>
        </Show>
    }
}

#[component]
fn CredentialTable(
    credentials: Vec<StreamCredentialDto>,
    version: RwSignal<u32>,
    revoked: RwSignal<Option<StreamRevocationDto>>,
) -> impl IntoView {
    let ready = use_hydrated();
    let confirm = RwSignal::new(None::<i64>);
    let revoke = Action::new(|id: &i64| {
        let id = *id;
        async move { revoke_stream_credential(id).await }
    });
    Effect::new(move |_| {
        if let Some(result) = revoke.value().get() {
            confirm.set(None);
            if let Ok(outcome) = result {
                revoked.set(Some(outcome));
            }
            version.update(|v| *v += 1);
        }
    });
    if credentials.is_empty() {
        return view! { <p class="vc-muted" id="camaccess-none">{move_tr!("camaccess-none")}</p> }
            .into_any();
    }
    view! {
        <div class="vc-card">
            <table class="vc-table" id="camaccess-table">
                <thead>
                    <tr>
                        <th>{move_tr!("camaccess-col-name")}</th>
                        <th>{move_tr!("camaccess-col-prefix")}</th>
                        <th>{move_tr!("camaccess-col-expires")}</th>
                        <th>{move_tr!("camaccess-col-used")}</th>
                        <th>{move_tr!("camaccess-col-state")}</th>
                        <th></th>
                    </tr>
                </thead>
                <tbody>
                    {credentials.into_iter().map(|c| {
                        let id = c.id;
                        let active = c.state == "active";
                        let (last_used, state) = (c.last_used_at.clone(), c.state.clone());
                        let used = move || if last_used.is_empty() { tr!("camaccess-never-used") } else { last_used.clone() };
                        view! {
                            <tr data-id=id data-state=c.state.clone()>
                                <td>{c.name.clone()}</td>
                                <td><code>{c.prefix.clone()}</code></td>
                                <td class="vc-muted">{c.expires_at.clone()}</td>
                                <td class="vc-muted">{used}</td>
                                <td>{move || state_text(&state)}</td>
                                <td>
                                    {active.then(|| view! {
                                        <Show
                                            when=move || confirm.get() == Some(id)
                                            fallback=move || view! {
                                                <button type="button" class="vc-btn vc-btn--ghost vc-btn--sm camaccess-revoke"
                                                    disabled=move || !ready.get()
                                                    on:click=move |_| confirm.set(Some(id))>
                                                    {move_tr!("camaccess-revoke")}
                                                </button>
                                            }
                                        >
                                            <button type="button" class="vc-btn vc-btn--danger vc-btn--sm camaccess-revoke-confirm"
                                                disabled=move || revoke.pending().get()
                                                on:click=move |_| { revoke.dispatch(id); }>
                                                {move_tr!("camaccess-revoke-confirm")}
                                            </button>
                                            <button type="button" class="vc-btn vc-btn--ghost vc-btn--sm" on:click=move |_| confirm.set(None)>
                                                {move_tr!("crud-cancel")}
                                            </button>
                                        </Show>
                                    })}
                                </td>
                            </tr>
                        }
                    }).collect_view()}
                </tbody>
            </table>
        </div>
    }
    .into_any()
}

/// What to type in the player: the user name and one address per camera (the password is the credential's).
#[component]
fn Addresses(username: String, cameras: Vec<CameraAddressDto>) -> impl IntoView {
    view! {
        <div class="vc-card" id="camaccess-addresses">
            <h3>{move_tr!("camaccess-addresses")}</h3>
            <p class="vc-muted">{move_tr!("camaccess-addresses-hint")}</p>
            <p>{move_tr!("camaccess-user")}" : "<code id="camaccess-username">{username}</code></p>
            {if cameras.is_empty() {
                view! { <p class="vc-muted">{move_tr!("camaccess-no-camera")}</p> }.into_any()
            } else {
                view! {
                    <table class="vc-table">
                        <tbody>
                            {cameras.into_iter().map(|c| view! {
                                <tr><td>{c.name}</td><td><code class="camaccess-address">{c.address}</code></td></tr>
                            }).collect_view()}
                        </tbody>
                    </table>
                }.into_any()
            }}
        </div>
    }
}
