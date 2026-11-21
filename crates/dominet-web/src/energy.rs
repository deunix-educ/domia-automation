// SPDX-License-Identifier: AGPL-3.0-or-later
//! Page d'energie `/automation/energy` (R4 lot 7, ADR 0036) : etat en direct (production solaire, consommation Linky, PAC, bilan net),
//! histogramme du bilan journalier (SVG calcule cote serveur, aucune bibliotheque JavaScript) et tableau des jours.
//! La meme donnee en JSON : `GET /api/energy/?days=N`.

use dominet_proto::{EnergyLiveDto, EnergyPageDto, Topic};
use leptos::prelude::*;
use leptos_fluent::{move_tr, tr};
use leptos_router::components::A;
use leptos_router::hooks::use_query_map;
use serde_json::Value;

use crate::guard::RequireAuth;
use crate::realtime::Realtime;

/// Durees proposees (jours).
const RANGES: [u32; 4] = [7, 14, 30, 90];

/// Les `days` derniers bilans (30 par defaut, 365 au plus), le graphique et les trois appareils du bilan avec leur dernier etat.
///
/// # Errors
/// Refus (401) sans session ; erreur interne si la base echoue.
#[server(name = EnergyPage, prefix = "/sf", endpoint = "energy/page")]
pub async fn energy_page(days: String) -> Result<EnergyPageDto, ServerFnError> {
    use dominet_domain::energy::chart_svg;
    use dominet_domain::repo::{DeviceRepo, EnergyRepo};
    use dominet_proto::EnergyRowDto;

    use crate::server_fns::{internal, require_user};

    let state = expect_context::<dominet_server::AppState>();
    require_user(&state).await?;
    let days = dominet_server::web::days_from(Some(&days)).max(1);
    let list = state
        .store
        .energy()
        .recent(days)
        .await
        .map_err(|e| internal("energy balances", &e))?;
    let live = |name: String| {
        let devices = state.store.devices();
        async move {
            Ok::<_, ServerFnError>(
                devices
                    .find_by_name(&name)
                    .await
                    .map_err(|e| internal("energy device", &e))?
                    .map(|d| EnergyLiveDto {
                        id: d.id.0,
                        name: d.friendly_name,
                        state: d.state,
                        available: d.available,
                    }),
            )
        }
    };
    Ok(EnergyPageDto {
        days,
        svg: chart_svg(&list),
        rows: list
            .iter()
            .map(|d| EnergyRowDto {
                date: d.day.to_string(),
                solar_wh: d.solar_wh,
                linky_wh: d.linky_total_wh(),
                pac_wh: d.pac_wh,
                net_wh: d.net_wh(),
            })
            .collect(),
        linky: live(state.config.energy.linky.clone()).await?,
        solar: live(state.config.energy.solar.clone()).await?,
        pac: live(state.config.energy.pac.clone()).await?,
    })
}

#[component]
pub fn EnergyPage() -> impl IntoView {
    view! {
        <RequireAuth>
            <main class="vc-container">
                <EnergyContent/>
            </main>
        </RequireAuth>
    }
}

/// Nombre lu dans l'etat d'un appareil (`None` si absent ou non numerique).
fn number(state: &Value, key: &str) -> Option<f64> {
    state.get(key).and_then(Value::as_f64)
}

/// Etat courant d'un appareil : le temps reel s'il est arrive, sinon celui de la page.
fn live_state(rt: Realtime, initial: Option<EnergyLiveDto>) -> Signal<Value> {
    let id = initial.as_ref().map(|d| d.id);
    if let Some(id) = id {
        Effect::new(move |_| rt.subscribe(Topic::Device(id)));
        on_cleanup(move || rt.unsubscribe(&Topic::Device(id)));
    }
    Signal::derive(move || {
        id.and_then(|i| rt.device(i))
            .map(|d| d.state)
            .or_else(|| initial.as_ref().map(|d| d.state.clone()))
            .unwrap_or(Value::Null)
    })
}

#[component]
fn EnergyContent() -> impl IntoView {
    let query = use_query_map();
    let days = move || query.with(|q| q.get("days").unwrap_or_else(|| "30".to_string()));
    let page = Resource::new(
        days,
        |d| async move { energy_page(d).await.map_err(|_| ()) },
    );
    view! {
        <div class="vc-page-header">
            <h2>{move_tr!("energy-title")}</h2>
            <nav class="vc-tabs" id="energy-ranges">
                {RANGES
                    .iter()
                    .map(|n| {
                        let n = *n;
                        view! {
                            <A
                                href=format!("/automation/energy?days={n}")
                                attr:class="vc-btn vc-btn--ghost vc-btn--sm"
                                attr:id=format!("energy-range-{n}")
                                attr:aria-current=move || (days() == n.to_string()).then_some("page")
                            >
                                {format!("{n} j")}
                            </A>
                        }
                    })
                    .collect_view()}
            </nav>
        </div>
        <Transition fallback=|| ()>
            {move || match page.get() {
                None => ().into_any(),
                Some(Err(())) => {
                    view! { <div class="vc-alert vc-alert--error" role="alert" id="energy-load-error">{move_tr!("crud-load-error")}</div> }
                        .into_any()
                }
                Some(Ok(p)) => view! { <EnergyBody page=p/> }.into_any(),
            }}
        </Transition>
    }
}

#[component]
fn EnergyBody(page: EnergyPageDto) -> impl IntoView {
    let rt = expect_context::<Realtime>();
    let solar = live_state(rt, page.solar.clone());
    let linky = live_state(rt, page.linky.clone());
    let pac = live_state(rt, page.pac.clone());
    let watts = |v: Option<f64>| v.map_or_else(|| "—".to_string(), |w| format!("{w:.0}"));
    // Bilan net en direct : production - consommation - PAC (comme la page de l'implementation de reference).
    let net = Signal::derive(move || {
        number(&solar.get(), "pac").unwrap_or(0.0)
            - number(&linky.get(), "PAPP").unwrap_or(0.0)
            - number(&pac.get(), "power_ab").unwrap_or(0.0)
    });
    let surplus = move || net.get() >= 0.0;
    let rows = page.rows.clone();
    let has_rows = !rows.is_empty();
    view! {
        <div class="vc-energy-live" id="energy-live">
            <div class="vc-card vc-energy-card vc-energy-card--solar">
                <div class="vc-muted">{move_tr!("energy-solar")}</div>
                <div class="vc-energy-value" id="energy-live-solar">{move || watts(number(&solar.get(), "pac"))}</div>
                <div class="vc-muted">"W"</div>
                <div class="vc-muted">
                    {move_tr!("energy-today")} " : "
                    <strong id="energy-live-qdy">
                        {move || {
                            number(&solar.get(), "qdy").map_or_else(|| "—".to_string(), |k| format!("{k:.1} kWh"))
                        }}
                    </strong>
                </div>
            </div>
            <div class="vc-card vc-energy-card vc-energy-card--linky">
                <div class="vc-muted">{move_tr!("energy-linky")}</div>
                <div class="vc-energy-value" id="energy-live-papp">{move || watts(number(&linky.get(), "PAPP"))}</div>
                <div class="vc-muted">"VA"</div>
                <div class="vc-muted" id="energy-live-ptec">
                    {move || {
                        linky
                            .get()
                            .get("PTEC")
                            .and_then(Value::as_str)
                            .map(|t| format!("{} : {t}", tr!("energy-tariff")))
                            .unwrap_or_default()
                    }}
                </div>
            </div>
            <div class="vc-card vc-energy-card vc-energy-card--pac">
                <div class="vc-muted">{move_tr!("energy-pac")}</div>
                <div class="vc-energy-value" id="energy-live-pac">{move || watts(number(&pac.get(), "power_ab"))}</div>
                <div class="vc-muted">"W"</div>
            </div>
            <div class="vc-card vc-energy-card vc-energy-card--net">
                <div class="vc-muted">{move_tr!("energy-net")}</div>
                <div
                    class="vc-energy-value"
                    class:vc-energy-pos=surplus
                    class:vc-energy-neg=move || !surplus()
                    id="energy-live-net"
                >
                    {move || format!("{:+.0}", net.get())}
                </div>
                <div class="vc-muted">"W"</div>
                <div class="vc-muted" id="energy-live-net-label">
                    {move || if net.get() >= 0.0 { tr!("energy-surplus") } else { tr!("energy-deficit") }}
                </div>
            </div>
        </div>
        <section class="vc-card">
            <h3 class="vc-card-title">{move_tr!("energy-daily")}</h3>
            {if has_rows {
                view! {
                    <div class="vc-energy-legend">
                        <span class="vc-energy-key vc-energy-key--solar">{move_tr!("energy-solar-short")}</span>
                        <span class="vc-energy-key vc-energy-key--linky">{move_tr!("energy-linky-short")}</span>
                        <span class="vc-energy-key vc-energy-key--pac">"PAC"</span>
                        <span class="vc-energy-key vc-energy-key--net">{move_tr!("energy-net-short")}</span>
                    </div>
                    <div class="vc-energy-chart" inner_html=page.svg.clone()></div>
                }
                    .into_any()
            } else {
                view! { <p class="vc-muted" id="energy-empty">{move_tr!("energy-empty")}</p> }.into_any()
            }}
        </section>
        {has_rows
            .then(|| {
                view! {
                    <div class="vc-card">
                        <table class="vc-table" id="energy-table">
                            <thead>
                                <tr>
                                    <th>{move_tr!("energy-col-date")}</th>
                                    <th>{move_tr!("energy-col-solar")}</th>
                                    <th>{move_tr!("energy-col-linky")}</th>
                                    <th>"PAC (Wh)"</th>
                                    <th>{move_tr!("energy-col-net")}</th>
                                </tr>
                            </thead>
                            <tbody>
                                {rows
                                    .into_iter()
                                    .rev()
                                    .map(|r| {
                                        let positive = r.net_wh >= 0;
                                        view! {
                                            <tr data-date=r.date.clone()>
                                                <td>{r.date.clone()}</td>
                                                <td class="vc-energy-pos">{r.solar_wh}</td>
                                                <td class="vc-energy-neg">{r.linky_wh}</td>
                                                <td class="vc-energy-pac">{r.pac_wh}</td>
                                                <td class:vc-energy-pos=positive class:vc-energy-neg=!positive>
                                                    {format!("{:+}", r.net_wh)}
                                                </td>
                                            </tr>
                                        }
                                    })
                                    .collect_view()}
                            </tbody>
                        </table>
                    </div>
                }
            })}
    }
}
