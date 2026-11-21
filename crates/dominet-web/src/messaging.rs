// SPDX-License-Identifier: AGPL-3.0-or-later
//! Messagerie interne (R5 lot 5, ADR 0041) : composer `/messaging/compose` (administrateurs), messages envoyes `/messaging` (administrateurs),
//! boite de reception `/messaging/inbox` (tout compte) et detail `/messaging/<id>`. L'envoi est asynchrone : la page de detail
//! suit le sujet `message:<id>` et recharge les statuts a chaque evenement.

use dominet_proto::{
    ComposeFormDto, ComposeInput, ComposeOutcome, DeliveryDto, FieldProblem, MessageDetailDto,
    MessageRowDto, Topic,
};
use leptos::prelude::*;
use leptos_fluent::{move_tr, tr};
use leptos_router::components::A;
use leptos_router::hooks::use_params_map;

use crate::android::shown;
use crate::browser::input_value;
use crate::crud::field_error;
use crate::guard::{RequireAdmin, RequireAuth};
use crate::layout::use_hydrated;
use crate::realtime::Realtime;

// ------------------------------------------------------------------ fonctions serveur

#[cfg(feature = "ssr")]
mod ssr {
    use std::collections::HashMap;

    use dominet_domain::messaging::{Delivery, Message};
    use dominet_domain::user::User;
    use dominet_proto::{DeliveryDto, MessageRowDto};

    pub fn name(users: &HashMap<i64, User>, id: i64) -> String {
        users.get(&id).map(User::display_name).unwrap_or_default()
    }

    /// Ligne de liste ; `mine` = l'envoi destine au compte connecte (boite de reception).
    pub fn row(m: &Message, users: &HashMap<i64, User>, mine: Option<&Delivery>) -> MessageRowDto {
        MessageRowDto {
            id: m.id.0,
            subject: m.subject.clone(),
            sender: name(users, m.sender.0),
            channel: m.channel.as_str().into(),
            status: m.status.as_str().into(),
            recipients: m.recipients_count,
            created_at: dominet_server::timefmt::rfc3339(m.created_at),
            my_email: mine
                .map(|d| d.email.as_str().to_string())
                .unwrap_or_default(),
            my_sms: mine.map(|d| d.sms.as_str().to_string()).unwrap_or_default(),
        }
    }

    pub fn delivery(d: &Delivery, users: &HashMap<i64, User>) -> DeliveryDto {
        DeliveryDto {
            id: d.id.0,
            recipient: name(users, d.recipient.0),
            email: d.email.as_str().into(),
            sms: d.sms.as_str().into(),
            error: d.error.clone(),
        }
    }
}

/// Ce qu'il faut pour composer : les comptes et l'etat des canaux.
///
/// # Errors
/// Refus (403) hors administrateur ; erreur interne si la base echoue.
#[server(name = MessagingComposeForm, prefix = "/sf", endpoint = "messaging/form")]
pub async fn compose_form() -> Result<ComposeFormDto, ServerFnError> {
    use dominet_domain::repo::{AndroidDeviceRepo, UserRepo};
    use dominet_proto::RecipientChoiceDto;

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    let mut users = state
        .store
        .users()
        .list()
        .await
        .map_err(|e| internal("accounts", &e))?;
    users.sort_by(|a, b| a.username.cmp(&b.username));
    let online = state
        .store
        .android_devices()
        .most_recent_online()
        .await
        .map_err(|e| internal("device online", &e))?;
    Ok(ComposeFormDto {
        users: users
            .iter()
            .filter(|u| u.is_active)
            .map(|u| RecipientChoiceDto {
                id: u.id.0,
                username: u.username.clone(),
                display_name: u.display_name(),
                has_email: !u.email.trim().is_empty(),
                has_mobile: !u.mobile.trim().is_empty(),
            })
            .collect(),
        email_ready: state.channels.email_configured(),
        android_online: online.is_some(),
    })
}

/// Enregistre le message et lance son envoi (en tache de fond) ; renvoie l'identifiant de la page de detail.
///
/// # Errors
/// Refus (403) hors administrateur ; erreur interne si la base echoue.
#[server(name = MessagingSend, prefix = "/sf", endpoint = "messaging/send", input = server_fn::codec::Json)]
pub async fn send_message(input: ComposeInput) -> Result<ComposeOutcome, ServerFnError> {
    use dominet_domain::Problem;
    use dominet_domain::messaging::MessageDraft;
    use dominet_domain::repo::UserRepo;

    use crate::server_fns::{field_problems, internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let user = require_admin(&state).await?;
    let users = state
        .store
        .users()
        .list()
        .await
        .map_err(|e| internal("accounts", &e))?;
    let picked = dominet_server::messaging::resolve_audience(&users, &input.audience, &input.users);
    let channel = dominet_server::messaging::parse_channel(&input.channel);
    let draft = MessageDraft {
        subject: input.subject.clone(),
        body: input.body.clone(),
        channel: channel.unwrap_or(dominet_domain::messaging::Channel::Email),
        recipients: picked.iter().map(|u| u.id).collect(),
    };
    let mut problems = draft.problems();
    if channel.is_none() {
        problems.push(("channel", Problem::Invalid));
    }
    if !problems.is_empty() {
        return Ok(ComposeOutcome::Invalid(field_problems(problems)));
    }
    let message = dominet_server::messaging::send(&state, user.id, &draft, &picked)
        .await
        .map_err(|e| internal("sending the message", &e))?;
    tracing::info!(
        user_id = user.id.0,
        message = message.id.0,
        recipients = picked.len(),
        "message composed"
    );
    Ok(ComposeOutcome::Sent {
        id: message.id.0,
        recipients: u32::try_from(picked.len()).unwrap_or(u32::MAX),
    })
}

/// Les 100 derniers messages envoyes.
///
/// # Errors
/// Refus (403) hors administrateur ; erreur interne si la base echoue.
#[server(name = MessagingSent, prefix = "/sf", endpoint = "messaging/sent")]
pub async fn sent_messages() -> Result<Vec<MessageRowDto>, ServerFnError> {
    use dominet_domain::repo::MessageRepo;

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    let users = dominet_server::messaging::users_by_id(&state)
        .await
        .map_err(|e| internal("accounts", &e))?;
    let list = state
        .store
        .messages()
        .list(100, 0)
        .await
        .map_err(|e| internal("messages", &e))?;
    Ok(list.iter().map(|m| ssr::row(m, &users, None)).collect())
}

/// Les 100 derniers messages recus par le compte.
///
/// # Errors
/// Refus (401) sans session ; erreur interne si la base echoue.
#[server(name = MessagingInbox, prefix = "/sf", endpoint = "messaging/inbox")]
pub async fn inbox_messages() -> Result<Vec<MessageRowDto>, ServerFnError> {
    use dominet_domain::repo::MessageRepo;

    use crate::server_fns::{internal, require_user};

    let state = expect_context::<dominet_server::AppState>();
    let user = require_user(&state).await?;
    let users = dominet_server::messaging::users_by_id(&state)
        .await
        .map_err(|e| internal("accounts", &e))?;
    let list = state
        .store
        .messages()
        .inbox(user.id, 100)
        .await
        .map_err(|e| internal("inbox", &e))?;
    Ok(list
        .iter()
        .map(|(m, d)| ssr::row(m, &users, Some(d)))
        .collect())
}

/// Un message : le tableau complet pour son expediteur et les administrateurs, son seul envoi pour un destinataire ; `None` si le
/// message n'existe pas ou ne concerne pas ce compte (meme reponse dans les deux cas).
///
/// # Errors
/// Refus (401) sans session ; erreur interne si la base echoue.
#[server(name = MessagingDetail, prefix = "/sf", endpoint = "messaging/detail")]
pub async fn message_detail(id: i64) -> Result<Option<MessageDetailDto>, ServerFnError> {
    use dominet_domain::messaging::MessageId;
    use dominet_domain::repo::MessageRepo;

    use crate::server_fns::{internal, require_user};

    let state = expect_context::<dominet_server::AppState>();
    let user = require_user(&state).await?;
    let messages = state.store.messages();
    let Some(m) = messages
        .get(MessageId(id))
        .await
        .map_err(|e| internal("message", &e))?
    else {
        return Ok(None);
    };
    let deliveries = messages
        .deliveries(m.id)
        .await
        .map_err(|e| internal("deliveries", &e))?;
    let mine = deliveries.iter().find(|d| d.recipient == user.id);
    let full_table = m.sender == user.id || user.role.is_admin();
    if !full_table && mine.is_none() {
        return Ok(None);
    }
    let users = dominet_server::messaging::users_by_id(&state)
        .await
        .map_err(|e| internal("accounts", &e))?;
    Ok(Some(MessageDetailDto {
        message: ssr::row(&m, &users, mine),
        body: m.body.clone(),
        full_table,
        deliveries: if full_table {
            let mut rows: Vec<DeliveryDto> = deliveries
                .iter()
                .map(|d| ssr::delivery(d, &users))
                .collect();
            rows.sort_by(|a, b| a.recipient.cmp(&b.recipient));
            rows
        } else {
            vec![]
        },
        mine: mine.map(|d| ssr::delivery(d, &users)),
    }))
}

// ------------------------------------------------------------------ vues

fn status_label(status: &str) -> String {
    match status {
        "pending" => tr!("messaging-status-pending"),
        "sent" => tr!("messaging-status-sent"),
        "failed" => tr!("messaging-status-failed"),
        _ => tr!("messaging-status-na"),
    }
}

fn channel_label(channel: &str) -> String {
    match channel {
        "sms" => tr!("messaging-channel-sms"),
        "both" => tr!("messaging-channel-both"),
        _ => tr!("messaging-channel-email"),
    }
}

fn message_state(status: &str) -> String {
    if status == "sent" {
        tr!("messaging-state-sent")
    } else {
        tr!("messaging-state-sending")
    }
}

#[component]
fn MessagingNav(current: &'static str, admin: bool) -> impl IntoView {
    let link = move |href: &'static str, key: &'static str, label: String| {
        view! {
            <A href=href attr:class="vc-btn vc-btn--ghost vc-btn--sm" attr:id=format!("messaging-tab-{key}") attr:aria-current=(key == current).then_some("page")>
                {label}
            </A>
        }
    };
    view! {
        <nav class="vc-tabs" id="messaging-tabs">
            {link("/messaging/inbox", "inbox", tr!("messaging-inbox"))}
            {admin.then(|| link("/messaging", "sent", tr!("messaging-sent")))}
            {admin.then(|| link("/messaging/compose", "compose", tr!("messaging-compose")))}
        </nav>
    }
}

#[component]
pub fn MessagingSentPage() -> impl IntoView {
    view! {
        <RequireAdmin>
            <main class="vc-container">
                <MessagingSent/>
            </main>
        </RequireAdmin>
    }
}

#[component]
fn MessagingSent() -> impl IntoView {
    let rt = expect_context::<Realtime>();
    let data = Resource::new(
        move || rt.messaging.get(),
        |_| async { sent_messages().await.map_err(|_| ()) },
    );
    view! {
        <div class="vc-page-header"><h2>{move_tr!("messaging-sent")}</h2></div>
        <MessagingNav current="sent" admin=true/>
        <Transition fallback=|| ()>
            {move || match data.get() {
                None => ().into_any(),
                Some(Err(())) => view! { <div class="vc-alert vc-alert--error" role="alert">{move_tr!("crud-load-error")}</div> }.into_any(),
                Some(Ok(rows)) if rows.is_empty() => view! { <p class="vc-muted" id="messaging-empty">{move_tr!("messaging-empty")}</p> }.into_any(),
                Some(Ok(rows)) => view! { <MessageTable rows=rows inbox=false/> }.into_any(),
            }}
        </Transition>
    }
}

#[component]
pub fn MessagingInboxPage() -> impl IntoView {
    view! {
        <RequireAuth>
            <main class="vc-container">
                <MessagingInbox/>
            </main>
        </RequireAuth>
    }
}

#[component]
fn MessagingInbox() -> impl IntoView {
    let rt = expect_context::<Realtime>();
    let layout = expect_context::<crate::layout::LayoutState>();
    let admin = move || {
        layout.data.get().is_some_and(|d| {
            d.user
                .as_ref()
                .is_some_and(|u| u.role == dominet_proto::RoleDto::Admin)
        })
    };
    let data = Resource::new(
        move || rt.messaging.get(),
        |_| async { inbox_messages().await.map_err(|_| ()) },
    );
    view! {
        <div class="vc-page-header"><h2>{move_tr!("messaging-inbox")}</h2></div>
        <MessagingNav current="inbox" admin=admin()/>
        <Transition fallback=|| ()>
            {move || match data.get() {
                None => ().into_any(),
                Some(Err(())) => view! { <div class="vc-alert vc-alert--error" role="alert">{move_tr!("crud-load-error")}</div> }.into_any(),
                Some(Ok(rows)) if rows.is_empty() => view! { <p class="vc-muted" id="messaging-empty">{move_tr!("messaging-inbox-empty")}</p> }.into_any(),
                Some(Ok(rows)) => view! { <MessageTable rows=rows inbox=true/> }.into_any(),
            }}
        </Transition>
    }
}

#[component]
fn MessageTable(rows: Vec<MessageRowDto>, inbox: bool) -> impl IntoView {
    view! {
        <div class="vc-card">
            <table class="vc-table" id="messaging-table">
                <thead>
                    <tr>
                        <th>{move_tr!("messaging-subject")}</th>
                        <th>{move_tr!("messaging-from")}</th>
                        <th>{move_tr!("messaging-channel")}</th>
                        <th>{if inbox { tr!("messaging-my-status") } else { tr!("messaging-recipients") }}</th>
                        <th>{move_tr!("messaging-date")}</th>
                    </tr>
                </thead>
                <tbody>
                    {rows
                        .into_iter()
                        .map(|m| {
                            let state = if inbox {
                                format!("{} / {}", status_label(&m.my_email), status_label(&m.my_sms))
                            } else {
                                format!("{} - {}", m.recipients, message_state(&m.status))
                            };
                            view! {
                                <tr data-id=m.id data-status=m.status.clone()>
                                    <td><A href=format!("/messaging/{}", m.id)>{m.subject.clone()}</A></td>
                                    <td>{m.sender.clone()}</td>
                                    <td>{channel_label(&m.channel)}</td>
                                    <td>{state}</td>
                                    <td>{shown(&m.created_at)}</td>
                                </tr>
                            }
                        })
                        .collect_view()}
                </tbody>
            </table>
        </div>
    }
}

// ------------------------------------------------------------------ composer

#[component]
pub fn MessagingComposePage() -> impl IntoView {
    view! {
        <RequireAdmin>
            <main class="vc-container">
                <MessagingCompose/>
            </main>
        </RequireAdmin>
    }
}

#[component]
fn MessagingCompose() -> impl IntoView {
    let ready = use_hydrated();
    let form = Resource::new(|| (), |()| async { compose_form().await.map_err(|_| ()) });
    let subject = NodeRef::<leptos::html::Input>::new();
    let body = NodeRef::<leptos::html::Textarea>::new();
    let channel = RwSignal::new("email".to_string());
    let everyone = RwSignal::new(false);
    let staff = RwSignal::new(false);
    let picked = RwSignal::new(Vec::<i64>::new());
    let problems = RwSignal::new(Vec::<FieldProblem>::new());
    let send = Action::new(|input: &ComposeInput| {
        let input = input.clone();
        async move { send_message(input).await }
    });
    let navigate = leptos_router::hooks::use_navigate();
    Effect::new(move |_| {
        if let Some(Ok(outcome)) = send.value().get() {
            match outcome {
                ComposeOutcome::Sent { id, .. } => {
                    navigate(
                        &format!("/messaging/{id}"),
                        leptos_router::NavigateOptions::default(),
                    );
                }
                ComposeOutcome::Invalid(p) => problems.set(p),
            }
        }
    });
    view! {
        <div class="vc-page-header"><h2>{move_tr!("messaging-compose")}</h2></div>
        <MessagingNav current="compose" admin=true/>
        <Transition fallback=|| ()>
            {move || match form.get() {
                None => ().into_any(),
                Some(Err(())) => view! { <div class="vc-alert vc-alert--error" role="alert">{move_tr!("crud-load-error")}</div> }.into_any(),
                Some(Ok(f)) => {
                    let email_ready = f.email_ready;
                    let android_online = f.android_online;
                    view! {
                        <div class="vc-card">
                            <form
                                id="messaging-form"
                                on:submit=move |ev: leptos::ev::SubmitEvent| {
                                    ev.prevent_default();
                                    let audience = if everyone.get_untracked() {
                                        "all"
                                    } else if staff.get_untracked() {
                                        "staff"
                                    } else {
                                        ""
                                    };
                                    send.dispatch(ComposeInput {
                                        subject: input_value(subject),
                                        body: body.get_untracked().map(|b| b.value()).unwrap_or_default(),
                                        channel: channel.get_untracked(),
                                        audience: audience.into(),
                                        users: picked.get_untracked(),
                                    });
                                }
                            >
                                <div class="vc-form-group">
                                    <label for="messaging-subject">{move_tr!("messaging-subject")}</label>
                                    <input id="messaging-subject" type="text" node_ref=subject maxlength="200"/>
                                    {field_error("subject", problems)}
                                </div>
                                <div class="vc-form-group">
                                    <label for="messaging-body">{move_tr!("messaging-body")}</label>
                                    <textarea id="messaging-body" node_ref=body rows="8"></textarea>
                                    {field_error("body", problems)}
                                </div>
                                <fieldset class="vc-form-group" id="messaging-channels">
                                    <legend>{move_tr!("messaging-channel")}</legend>
                                    {["email", "sms", "both"]
                                        .into_iter()
                                        .map(|c| {
                                            view! {
                                                <label>
                                                    <input
                                                        type="radio"
                                                        name="messaging-channel"
                                                        id=format!("messaging-channel-{c}")
                                                        value=c
                                                        checked=c == "email"
                                                        on:change=move |_| channel.set(c.to_string())
                                                    />
                                                    " " {channel_label(c)} " "
                                                </label>
                                            }
                                        })
                                        .collect_view()}
                                    {field_error("channel", problems)}
                                    {(!email_ready).then(|| view! { <p class="vc-muted" id="messaging-no-smtp">{move_tr!("messaging-no-smtp")}</p> })}
                                    {(!android_online).then(|| view! { <p class="vc-muted" id="messaging-no-android">{move_tr!("messaging-no-android")}</p> })}
                                </fieldset>
                                <fieldset class="vc-form-group" id="messaging-audience">
                                    <legend>{move_tr!("messaging-recipients")}</legend>
                                    <label>
                                        <input type="checkbox" id="messaging-all" on:change=move |ev| everyone.set(leptos::prelude::event_target_checked(&ev))/>
                                        " " {move_tr!("messaging-all")} " "
                                    </label>
                                    <label>
                                        <input type="checkbox" id="messaging-staff" on:change=move |ev| staff.set(leptos::prelude::event_target_checked(&ev))/>
                                        " " {move_tr!("messaging-staff")} " "
                                    </label>
                                    <ul class="vc-list" id="messaging-users">
                                        {f
                                            .users
                                            .into_iter()
                                            .map(|u| {
                                                let id = u.id;
                                                let hint = format!(
                                                    "{}{}",
                                                    if u.has_email { "" } else { " (no e-mail)" },
                                                    if u.has_mobile { "" } else { " (no mobile)" },
                                                );
                                                view! {
                                                    <li>
                                                        <label>
                                                            <input
                                                                type="checkbox"
                                                                id=format!("messaging-user-{}", u.username)
                                                                on:change=move |ev| {
                                                                    let on = leptos::prelude::event_target_checked(&ev);
                                                                    picked.update(|p| {
                                                                        p.retain(|x| *x != id);
                                                                        if on {
                                                                            p.push(id);
                                                                        }
                                                                    });
                                                                }
                                                            />
                                                            " " {u.display_name.clone()} <span class="vc-muted">{hint}</span>
                                                        </label>
                                                    </li>
                                                }
                                            })
                                            .collect_view()}
                                    </ul>
                                    {field_error("recipients", problems)}
                                </fieldset>
                                <button type="submit" id="messaging-send" class="vc-btn vc-btn--primary" disabled=move || !ready.get() || send.pending().get()>
                                    {move_tr!("messaging-send")}
                                </button>
                            </form>
                        </div>
                    }
                        .into_any()
                }
            }}
        </Transition>
    }
}

// ------------------------------------------------------------------ detail

#[component]
pub fn MessagingDetailPage() -> impl IntoView {
    view! {
        <RequireAuth>
            <main class="vc-container">
                <MessagingDetail/>
            </main>
        </RequireAuth>
    }
}

#[component]
fn MessagingDetail() -> impl IntoView {
    let rt = expect_context::<Realtime>();
    let params = use_params_map();
    let id = move || params.with(|p| p.get("id").and_then(|i| i.parse::<i64>().ok()));
    let data = Resource::new(
        move || (id(), rt.messaging.get()),
        |(id, _)| async move {
            match id {
                Some(id) => message_detail(id).await.map_err(|_| ()),
                None => Ok(None),
            }
        },
    );
    // Les statuts changent pendant l'envoi : on suit le sujet du message tant que la page est ouverte.
    let held = StoredValue::new(None::<i64>);
    Effect::new(move |_| {
        let now = id();
        if let Some(old) = held.get_value().filter(|o| Some(*o) != now) {
            rt.unsubscribe(&Topic::Message(old));
        }
        if let Some(n) = now.filter(|n| held.get_value() != Some(*n)) {
            rt.subscribe(Topic::Message(n));
        }
        held.set_value(now);
    });
    on_cleanup(move || {
        if let Some(old) = held.get_value() {
            rt.unsubscribe(&Topic::Message(old));
        }
    });
    view! {
        <div class="vc-page-header">
            <h2>{move_tr!("messaging-detail")}</h2>
            <div class="vc-header-actions">
                <A href="/messaging/inbox" attr:class="vc-btn vc-btn--ghost">{move_tr!("messaging-inbox")}</A>
            </div>
        </div>
        <Transition fallback=|| ()>
            {move || match data.get() {
                None => ().into_any(),
                Some(Err(())) => view! { <div class="vc-alert vc-alert--error" role="alert">{move_tr!("crud-load-error")}</div> }.into_any(),
                Some(Ok(None)) => view! { <p class="vc-alert vc-alert--error" id="messaging-not-found">{move_tr!("messaging-not-found")}</p> }.into_any(),
                Some(Ok(Some(d))) => view! { <DetailBody d=d/> }.into_any(),
            }}
        </Transition>
    }
}

fn delivery_row(d: &DeliveryDto) -> impl IntoView + use<> {
    view! {
        <tr data-id=d.id data-email=d.email.clone() data-sms=d.sms.clone()>
            <td>{d.recipient.clone()}</td>
            <td class="messaging-email">{status_label(&d.email)}</td>
            <td class="messaging-sms">{status_label(&d.sms)}</td>
            <td class="vc-muted">{d.error.clone()}</td>
        </tr>
    }
}

#[component]
fn DetailBody(d: MessageDetailDto) -> impl IntoView {
    let m = d.message;
    view! {
        <div class="vc-card" id="messaging-message" data-status=m.status.clone()>
            <h3 id="messaging-title">{m.subject.clone()}</h3>
            <p class="vc-muted">
                {m.sender.clone()} " - " {shown(&m.created_at)} " - " {channel_label(&m.channel)} " - "
                <span id="messaging-state">{message_state(&m.status)}</span>
            </p>
            <div id="messaging-text" class="messaging-body">{d.body}</div>
        </div>
        {if d.full_table {
            view! {
                <div class="vc-card">
                    <table class="vc-table" id="messaging-deliveries">
                        <thead>
                            <tr>
                                <th>{move_tr!("messaging-recipient")}</th>
                                <th>{move_tr!("messaging-email")}</th>
                                <th>{move_tr!("messaging-sms")}</th>
                                <th>{move_tr!("messaging-error")}</th>
                            </tr>
                        </thead>
                        <tbody>{d.deliveries.iter().map(delivery_row).collect_view()}</tbody>
                    </table>
                </div>
            }
                .into_any()
        } else {
            d.mine
                .map(|mine| {
                    view! {
                        <p id="messaging-mine" data-email=mine.email.clone() data-sms=mine.sms.clone()>
                            {move_tr!("messaging-my-status")} " : " {status_label(&mine.email)} " / " {status_label(&mine.sms)}
                        </p>
                    }
                })
                .into_any()
        }}
    }
}
