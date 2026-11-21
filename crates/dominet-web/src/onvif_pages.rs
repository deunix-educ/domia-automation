// SPDX-License-Identifier: AGPL-3.0-or-later
//! Decouverte ONVIF, ajout par adresse IP et synchronisation ONVIF : `/surveillance/discovery` (R6 lot 3, ADR 0045). Administrateurs.

use dominet_proto::{AddByIpOutcome, AddCameraByIp, OnvifSyncOutcome, Topic};
use leptos::prelude::*;
use leptos_fluent::move_tr;
use leptos_router::components::A;

use crate::camera_fns::{add_camera_by_ip, list_cameras, start_discovery, sync_camera_onvif};
use crate::guard::RequireAdmin;
use crate::realtime::Realtime;

/// Compteur partage : une camera vient d'etre ajoutee, la liste des cameras ONVIF se relit.
#[derive(Clone, Copy)]
struct Refresh(RwSignal<u32>);

#[component]
pub fn DiscoveryPage() -> impl IntoView {
    provide_context(Refresh(RwSignal::new(0)));
    view! {
        <RequireAdmin>
            <main class="vc-container">
                <div class="vc-page-header"><h2>{move_tr!("discovery-title")}</h2></div>
                <p class="vc-btn-row">
                    <A href="/surveillance" attr:class="vc-btn vc-btn--ghost">{move_tr!("surveillance-title")}</A>
                    <A href="/admin/cameras" attr:class="vc-btn vc-btn--ghost">{move_tr!("cameras-title")}</A>
                </p>
                <Discovery/>
                <ManualAdd/>
                <OnvifCameras/>
            </main>
        </RequireAdmin>
    }
}

fn added_message(o: &AddByIpOutcome) -> (bool, String) {
    match o {
        AddByIpOutcome::Added {
            name,
            created,
            model,
            ..
        } => (
            true,
            if *created {
                leptos_fluent::tr!("discovery-added", {"name" => name.clone(), "model" => model.clone()})
            } else {
                leptos_fluent::tr!("discovery-already", {"name" => name.clone()})
            },
        ),
        AddByIpOutcome::Invalid(_) => (false, leptos_fluent::tr!("discovery-invalid-ip")),
        AddByIpOutcome::NoOnvif => (false, leptos_fluent::tr!("discovery-no-onvif")),
    }
}

#[component]
fn Discovery() -> impl IntoView {
    let rt = expect_context::<Realtime>();
    rt.subscribe(Topic::Discovery);
    on_cleanup(move || rt.unsubscribe(&Topic::Discovery));
    let view = rt.discovery;
    // Demande envoyee, rien recu au bout de quelques secondes : probablement aucun role `discovery`.
    let waiting = RwSignal::new(false);
    let silent = RwSignal::new(false);
    let start = Action::new(move |(): &()| async move {
        waiting.set(true);
        silent.set(false);
        let sent = start_discovery().await.is_ok();
        if sent {
            set_timeout(
                move || {
                    if waiting.get_untracked()
                        && !view.get_untracked().scanning
                        && !view.get_untracked().done
                    {
                        silent.set(true);
                    }
                },
                std::time::Duration::from_secs(5),
            );
        }
        sent
    });
    Effect::new(move |_| {
        let v = view.get();
        if v.scanning || v.done || v.error.is_some() {
            waiting.set(false);
            silent.set(false);
        }
    });
    let add_result = RwSignal::new(None::<(bool, String)>);
    let refresh = expect_context::<Refresh>().0;
    let add = Action::new(move |(ip, port): &(String, u16)| {
        let input = AddCameraByIp {
            ip: ip.clone(),
            onvif_port: Some(*port),
            user: String::new(),
            pass: String::new(),
        };
        async move {
            let out = add_camera_by_ip(input).await.map(|o| added_message(&o));
            add_result.set(Some(
                out.unwrap_or((false, leptos_fluent::tr!("crud-load-error"))),
            ));
            refresh.update(|v| *v += 1);
        }
    });
    view! {
        <section id="discovery" class="vc-card">
            <h3 class="vc-card-title">{move_tr!("discovery-network")}</h3>
            <p>{move_tr!("discovery-network-hint")}</p>
            <button type="button" class="vc-btn vc-btn--primary" id="discovery-start" on:click=move |_| { start.dispatch(()); }
                prop:disabled=move || view.get().scanning>
                {move_tr!("discovery-start")}
            </button>
            {move || view.get().scanning.then(|| view! { <p id="discovery-scanning" role="status">{move_tr!("discovery-scanning")}</p> })}
            {move || silent.get().then(|| view! { <div class="vc-alert vc-alert--error" role="alert" id="discovery-silent">{move_tr!("discovery-silent")}</div> })}
            {move || view.get().error.map(|e| view! { <div class="vc-alert vc-alert--error" role="alert" id="discovery-error">{e}</div> })}
            {move || {
                let v = view.get();
                (v.done && v.found.is_empty()).then(|| view! { <p id="discovery-none">{move_tr!("discovery-none")}</p> })
            }}
            <ul id="discovery-found" class="vc-list">
                <For each=move || view.get().found key=|d| d.ip.clone() let:d>
                    <li data-ip=d.ip.clone()>
                        <strong>{d.ip.clone()}</strong>{format!(":{}", d.port)}
                        " "{d.name.clone()}" "{d.hardware.clone()}" "
                        {if d.known {
                            view! { <span class="vc-badge">{move_tr!("discovery-known")}</span> }.into_any()
                        } else {
                            let (ip, port) = (d.ip.clone(), d.port);
                            view! {
                                <button type="button" class="vc-btn discovery-add" on:click=move |_| { add.dispatch((ip.clone(), port)); }>
                                    {move_tr!("discovery-add")}
                                </button>
                            }.into_any()
                        }}
                    </li>
                </For>
            </ul>
            {move || add_result.get().map(|(ok, msg)| {
                let class = if ok { "vc-alert vc-alert--success" } else { "vc-alert vc-alert--error" };
                view! { <div class=class id="discovery-add-result" role="status">{msg}</div> }
            })}
        </section>
    }
}

#[component]
fn ManualAdd() -> impl IntoView {
    let ip = RwSignal::new(String::new());
    let port = RwSignal::new(String::new());
    let user = RwSignal::new(String::new());
    let pass = RwSignal::new(String::new());
    let result = RwSignal::new(None::<(bool, String)>);
    let refresh = expect_context::<Refresh>().0;
    let add = Action::new(move |(): &()| {
        let input = AddCameraByIp {
            ip: ip.get_untracked().trim().to_string(),
            onvif_port: port.get_untracked().trim().parse().ok(),
            user: user.get_untracked(),
            pass: pass.get_untracked(),
        };
        async move {
            let out = add_camera_by_ip(input).await.map(|o| added_message(&o));
            if matches!(out, Ok((true, _))) {
                pass.set(String::new());
            }
            result.set(Some(
                out.unwrap_or((false, leptos_fluent::tr!("crud-load-error"))),
            ));
            refresh.update(|v| *v += 1);
        }
    });
    view! {
        <section id="manual-add" class="vc-card">
            <h3 class="vc-card-title">{move_tr!("discovery-manual")}</h3>
            <p>{move_tr!("discovery-manual-hint")}</p>
            <form on:submit=move |ev| { ev.prevent_default(); add.dispatch(()); }>
                <div class="vc-form-group">
                    <label for="manual-ip">{move_tr!("cameras-ip")}</label>
                    <input id="manual-ip" type="text" required maxlength="64" prop:value=move || ip.get()
                        on:input=move |e| ip.set(event_target_value(&e))/>
                </div>
                <div class="vc-form-group">
                    <label for="manual-port">{move_tr!("cameras-onvif-port")}</label>
                    <input id="manual-port" type="number" min="1" max="65535" prop:value=move || port.get()
                        on:input=move |e| port.set(event_target_value(&e))/>
                </div>
                <div class="vc-form-group">
                    <label for="manual-user">{move_tr!("cameras-rtsp-user")}</label>
                    <input id="manual-user" type="text" autocomplete="off" maxlength="128" prop:value=move || user.get()
                        on:input=move |e| user.set(event_target_value(&e))/>
                </div>
                <div class="vc-form-group">
                    <label for="manual-pass">{move_tr!("cameras-rtsp-pass")}</label>
                    <input id="manual-pass" type="password" autocomplete="new-password" maxlength="128" prop:value=move || pass.get()
                        on:input=move |e| pass.set(event_target_value(&e))/>
                </div>
                <div class="vc-form-actions">
                    <button type="submit" class="vc-btn vc-btn--primary" id="manual-submit">{move_tr!("discovery-manual-add")}</button>
                </div>
            </form>
            {move || result.get().map(|(ok, msg)| {
                let class = if ok { "vc-alert vc-alert--success" } else { "vc-alert vc-alert--error" };
                view! { <div class=class id="manual-result" role="status">{msg}</div> }
            })}
        </section>
    }
}

#[component]
fn OnvifCameras() -> impl IntoView {
    let version = expect_context::<Refresh>().0;
    let list = Resource::new(
        move || version.get(),
        |_| async { list_cameras().await.unwrap_or_default() },
    );
    let outcome = RwSignal::new(None::<(i64, OnvifSyncOutcome)>);
    let sync = Action::new(move |id: &i64| {
        let id = *id;
        async move {
            let out = sync_camera_onvif(id)
                .await
                .unwrap_or(OnvifSyncOutcome::Failed(String::new()));
            outcome.set(Some((id, out)));
            version.update(|v| *v += 1);
        }
    });
    view! {
        <section id="onvif-cameras" class="vc-card">
            <h3 class="vc-card-title">{move_tr!("discovery-sync-title")}</h3>
            <Transition fallback=|| ()>
                <ul class="vc-list">
                    {move || list.get().unwrap_or_default().into_iter().filter(|c| c.onvif).map(|c| {
                        let id = c.id;
                        let shown = (c.manufacturer.clone(), c.model.clone(), c.firmware.clone());
                        view! {
                            <li data-camera=c.uuid.clone()>
                                <strong>{c.name.clone()}</strong>
                                " "{format!("{}:{}", c.ip, c.onvif_port)}" "
                                <span class="camera-info">{format!("{} {} {}", shown.0, shown.1, shown.2).trim().to_string()}</span>
                                " "
                                <button type="button" class="vc-btn camera-onvif-sync" on:click=move |_| { sync.dispatch(id); }>
                                    {move_tr!("discovery-sync")}
                                </button>
                                {move || outcome.get().filter(|(i, _)| *i == id).map(|(_, o)| sync_message(&o))}
                            </li>
                        }
                    }).collect_view()}
                </ul>
            </Transition>
        </section>
    }
}

fn sync_message(o: &OnvifSyncOutcome) -> impl IntoView + use<> {
    let (ok, text) = match o {
        OnvifSyncOutcome::Done {
            model,
            main_token,
            ptz,
            stream_uri,
            ..
        } => (
            true,
            leptos_fluent::tr!("discovery-sync-done", {
                "model" => model.clone(),
                "token" => main_token.clone(),
                "ptz" => if *ptz { leptos_fluent::tr!("crud-yes") } else { leptos_fluent::tr!("crud-no") },
                "uri" => stream_uri.clone()
            }),
        ),
        OnvifSyncOutcome::Disabled => (false, leptos_fluent::tr!("discovery-sync-disabled")),
        OnvifSyncOutcome::NotAuthorized => (false, leptos_fluent::tr!("discovery-sync-denied")),
        OnvifSyncOutcome::NotFound => (false, leptos_fluent::tr!("discovery-sync-notfound")),
        OnvifSyncOutcome::Failed(m) => (
            false,
            format!("{} {m}", leptos_fluent::tr!("discovery-sync-failed")),
        ),
    };
    let class = if ok {
        "vc-alert vc-alert--success onvif-result"
    } else {
        "vc-alert vc-alert--error onvif-result"
    };
    view! { <div class=class role="status">{text}</div> }
}
