// SPDX-License-Identifier: AGPL-3.0-or-later
//! Mise en page : barre laterale (navigation dynamique par role, repli, menu mobile, plein ecran, langue,
//! deconnexion) et zone de contenu.
//!
//! Le decalage du contenu ne depend d'aucune classe posee par du JavaScript : le CSS l'obtient de la fraternite des
//! elements (`.vc-sidebar + .vc-main-wrap`), donc le HTML rendu par le serveur est deja correct (pas de saut a l'hydratation).

use dominet_proto::{LayoutDto, NavItemDto, Topic, UserDto};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_fluent::{I18n, move_tr, tr};
use leptos_router::components::A;
use leptos_router::hooks::use_navigate;

use crate::realtime::{Realtime, RealtimeState};
use crate::server_fns::{layout_data, logout};
use crate::{browser, chrome};

/// Compteur a incrementer pour recharger la navigation (connexion, deconnexion, modification de la navigation).
#[derive(Clone, Copy, Debug)]
pub struct LayoutRefresh(pub RwSignal<u32>);

/// Navigation et compte courant, partages avec les pages (gardes, page de connexion) pour ne les charger qu'une fois.
/// Le second champ vaut vrai pendant un depart volontaire (voir [`leave_session`]).
#[derive(Clone, Copy, Debug)]
pub struct LayoutState {
    pub data: Resource<LayoutDto>,
    /// Vrai pendant un depart volontaire (voir [`leave_session`]).
    pub leaving: RwSignal<bool>,
}

/// Ce que les gardes de pages savent du visiteur.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthStatus {
    Anonymous,
    User,
    Admin,
}

pub(crate) fn status_of(d: &LayoutDto) -> AuthStatus {
    match &d.user {
        None => AuthStatus::Anonymous,
        Some(u) if u.role == dominet_proto::RoleDto::Admin => AuthStatus::Admin,
        Some(_) => AuthStatus::User,
    }
}

/// Apres la fermeture de toutes les sessions du compte (deconnexion, changement de mot de passe) : va sur `target`
/// et declare l'etat anonyme **dans la meme foulee**, avant le rechargement reseau.
///
/// Sans cela, la garde de la page quittee redirigerait vers `/login?next=...` (ce qui ecraserait `target`), et la page
/// de connexion, voyant encore le compte, renverrait vers l'accueil.
///
/// Les contextes sont passes en parametres : apres un `await` le proprietaire reactif n'est plus garanti.
pub fn leave_session(
    navigate: impl Fn(&str, leptos_router::NavigateOptions),
    target: &str,
    state: LayoutState,
    refresh: LayoutRefresh,
) {
    // Les gardes ne redirigent pas pendant un depart volontaire (la navigation du routeur n'est pas instantanee) ;
    // la page de connexion remet l'indicateur a zero.
    state.leaving.set(true);
    navigate(target, leptos_router::NavigateOptions::default());
    state.data.set(Some(LayoutDto::default()));
    refresh.0.update(|n| *n += 1);
}

/// Vrai des que l'interface est hydratee (toujours faux pendant le rendu serveur). Sert a garder inactifs les boutons
/// d'envoi d'un formulaire tant que le navigateur ne peut pas le traiter : sinon une soumission trop rapide partirait
/// comme un formulaire HTML ordinaire (page rechargee, saisie perdue).
pub fn use_hydrated() -> ReadSignal<bool> {
    let (ready, set_ready) = signal(false);
    // Les effets ne s'executent que dans le navigateur, apres l'hydratation.
    Effect::new(move |_| set_ready.set(true));
    ready
}

/// Selecteur de langue (francais / anglais), sans rechargement.
#[component]
pub fn LanguageSwitcher() -> impl IntoView {
    let i18n = expect_context::<I18n>();
    view! {
        <div class="vc-lang">
            {move || {
                i18n.languages
                    .iter()
                    .map(|lang| {
                        view! {
                            <button
                                type="button"
                                class="vc-btn"
                                aria-pressed=move || (i18n.language.get() == *lang).to_string()
                                on:click=move |_| i18n.language.set(lang)
                            >
                                {lang.name}
                            </button>
                        }
                    })
                    .collect::<Vec<_>>()
            }}
        </div>
    }
}

/// Enveloppe les pages : barre laterale pour un compte connecte, contenu et pied de page pour tous.
#[component]
pub fn Layout(children: Children) -> impl IntoView {
    let refresh = expect_context::<LayoutRefresh>();
    // Ressource *bloquante* : le serveur attend la reponse avant d'envoyer le HTML, donc la barre laterale est dans
    // la page initiale (pas d'insertion tardive qui decalerait le contenu).
    let data = Resource::new_blocking(
        move || refresh.0.get(),
        |_| async { layout_data().await.unwrap_or_default() },
    );
    provide_context(LayoutState {
        data,
        leaving: RwSignal::new(false),
    });
    let rt = Realtime::new();
    provide_context(rt);
    // Connexion temps reel : ouverte tant qu'un compte est connecte. L'effet se relance a chaque rechargement des
    // donnees, d'ou la comparaison avec l'etat courant (aucune action si rien ne change).
    #[cfg(feature = "hydrate")]
    {
        let connection = StoredValue::new_local(None::<crate::realtime::client::Handle>);
        Effect::new(move |_| {
            let signed_in = data.get().is_some_and(|d| d.user.is_some());
            connection.update_value(|c| match (signed_in, c.is_some()) {
                (true, false) => *c = Some(crate::realtime::client::start(rt)),
                (false, true) => {
                    if let Some(handle) = c.take() {
                        handle.stop();
                    }
                }
                _ => {}
            });
        });
        // Les composants demandent ou abandonnent des sujets : on envoie les differences au serveur.
        Effect::new(move |_| {
            rt.topics.track();
            connection.with_value(|c| {
                if let Some(handle) = c {
                    handle.sync();
                }
            });
        });
    }
    let collapsed = RwSignal::new(false);
    let mobile_open = RwSignal::new(false);
    // Apres l'hydratation seulement : restaure l'etat replie memorise (aucun effet cote serveur).
    Effect::new(move |_| {
        if let Some(stored) = browser::stored_collapsed() {
            collapsed.set(stored);
        }
    });

    view! {
        <Transition fallback=|| ()>
            {move || {
                data.get()
                    .and_then(|d| {
                        d.user
                            .clone()
                            .map(|user| {
                                view! {
                                    <Sidebar
                                        user
                                        nav_top=d.nav_top.clone()
                                        nav_bottom=d.nav_bottom.clone()
                                        collapsed
                                        mobile_open
                                    />
                                }
                            })
                    })
            }}
        </Transition>
        <div class="vc-main-wrap" on:click=move |_| mobile_open.set(false)>
            <main class="vc-main">{children()}</main>
            <footer class="vc-footer">
                <span>"(C) DomiNet"</span>
            </footer>
        </div>
        <Toasts/>
    }
}

/// Une entree de navigation : lien interne (routage cote navigateur) ou externe (nouvel onglet si demande).
fn nav_link(item: NavItemDto) -> impl IntoView {
    let icon = view! { <span class="vc-nav-icon" inner_html=item.icon_svg.clone()></span> };
    // Un lien interne ouvert dans un nouvel onglet n'est pas une navigation du routeur : lien HTML ordinaire. De meme pour
    // une page servie hors de l'interface (`/docs/`, `/grafana/`) : le routeur ne la connait pas et afficherait « Page
    // introuvable » ; `rel="external"` lui dit de laisser le navigateur charger la page.
    if item.url.starts_with('/')
        && item.url_target != "_blank"
        && !crate::nav::leaves_app(&item.url)
    {
        view! {
            <A href=item.url.clone() attr:class="vc-nav-link">
                {icon}
                <span>{item.label.clone()}</span>
            </A>
        }
        .into_any()
    } else {
        view! {
            <a class="vc-nav-link" href=item.url.clone() target=item.url_target.clone() rel="external noopener noreferrer">
                {icon}
                <span>{item.label.clone()}</span>
            </a>
        }
        .into_any()
    }
}

/// Etat de la connexion temps reel (point colore + libelle).
#[component]
fn RealtimeIndicator() -> impl IntoView {
    let rt = expect_context::<Realtime>();
    view! {
        <div class="vc-rt" id="vc-rt-state" data-state=move || rt.state.get().as_str()>
            <span class="vc-rt-dot"></span>
            <span>
                {move || match rt.state.get() {
                    RealtimeState::Online => tr!("rt-online"),
                    RealtimeState::Connecting => tr!("rt-connecting"),
                    RealtimeState::Offline => tr!("rt-offline"),
                }}
            </span>
        </div>
    }
}

/// Une jauge : icone, barre et pourcentage. `pct` vaut `None` sans mesure (texte « … », barre vide).
#[component]
fn Gauge(
    id: &'static str,
    icon: &'static str,
    kind: &'static str,
    pct: Signal<Option<f64>>,
    tip: Signal<String>,
) -> impl IntoView {
    view! {
        <div class="vc-stat-row" id=format!("vc-stat-{id}") title=move || tip.get()>
            <span class="vc-stat-icon" inner_html=icon></span>
            <div class="vc-stat-track">
                // La largeur passe par la CSSOM (autorisee par la CSP), jamais par un attribut `style` rendu.
                <div
                    class=format!("vc-stat-fill vc-stat-fill--{kind}")
                    class:vc-stat-fill--warn=move || pct.get().is_some_and(|p| p >= 90.0)
                    id=format!("vc-stat-{id}-bar")
                    style:width=move || pct.get().map(|p| format!("{p}%"))
                ></div>
            </div>
            <span class="vc-stat-pct" id=format!("vc-stat-{id}-pct")>
                {move || pct.get().map_or_else(|| "…".to_string(), |p| format!("{p:.0}%"))}
            </span>
        </div>
    }
}

/// Jauges de la machine (processeur, memoire, disque, temperature), alimentees par le sujet `system`. Tant qu'aucune
/// mesure n'est arrivee, ou si la connexion est coupee, les valeurs restent a « … » plutot que d'afficher du perime.
#[component]
fn SystemGauges() -> impl IntoView {
    let rt = expect_context::<Realtime>();
    Effect::new(move |_| rt.subscribe(Topic::System));
    on_cleanup(move || rt.unsubscribe(&Topic::System));
    let stats = move || {
        (rt.state.get() == RealtimeState::Online)
            .then(|| rt.system.get())
            .flatten()
    };
    let cpu_pct = Signal::derive(move || stats().map(|s| s.cpu.percent));
    let cpu_tip = Signal::derive(move || match stats() {
        Some(s) => {
            tr!("stats-cpu-title", {"percent" => s.cpu.percent, "cores" => s.cpu.count})
        }
        None => tr!("stats-cpu"),
    });
    let mem_pct = Signal::derive(move || stats().map(|s| s.memory.percent));
    let mem_tip = Signal::derive(move || match stats() {
        Some(s) => tr!("stats-mem-title", {
            "used" => s.memory.used_gb, "total" => s.memory.total_gb, "percent" => s.memory.percent
        }),
        None => tr!("stats-mem"),
    });
    let disk = move || stats().and_then(|s| s.disk);
    let disk_pct = Signal::derive(move || disk().map(|d| d.percent));
    let disk_tip = Signal::derive(move || match disk() {
        Some(d) => tr!("stats-disk-title", {
            "used" => d.used_gb, "total" => d.total_gb, "percent" => d.percent
        }),
        None => tr!("stats-disk"),
    });
    let temperature = move || stats().and_then(|s| s.temperature);
    view! {
        <div class="vc-sidebar-stats" id="vc-sys-stats" aria-label=move || tr!("stats-title")>
            <Gauge id="cpu" icon=chrome::CPU kind="cpu" pct=cpu_pct tip=cpu_tip/>
            <Gauge id="mem" icon=chrome::MEMORY kind="mem" pct=mem_pct tip=mem_tip/>
            <Gauge id="disk" icon=chrome::DISK kind="disk" pct=disk_pct tip=disk_tip/>
            {move || {
                temperature()
                    .map(|t| {
                        view! {
                            <div class="vc-stat-row" id="vc-stat-temp" title=move || tr!("stats-temp")>
                                <span class="vc-stat-icon" inner_html=chrome::THERMOMETER></span>
                                <span class="vc-stat-temp-label">{move || tr!("stats-temp")}</span>
                                <span class="vc-stat-pct vc-stat-temp-value" id="vc-stat-temp-value">
                                    {format!("{t:.0} °C")}
                                </span>
                            </div>
                        }
                    })
            }}
        </div>
    }
}

/// Notifications recues en temps reel, empilees en bas a droite (texte seulement : jamais de HTML).
#[component]
fn Toasts() -> impl IntoView {
    let rt = expect_context::<Realtime>();
    view! {
        <div class="vc-toasts" id="vc-toasts" aria-live="polite">
            <For each=move || rt.toasts.get() key=|t| t.id let:toast>
                <div class=format!("vc-toast vc-toast--{}", level_class(toast.level)) role="status">
                    <span class="vc-toast-text">{toast.text.clone()}</span>
                    <button
                        type="button"
                        class="vc-toast-close"
                        aria-label=move || tr!("rt-dismiss")
                        on:click=move |_| rt.dismiss(toast.id)
                    >
                        "x"
                    </button>
                </div>
            </For>
        </div>
    }
}

fn level_class(level: dominet_proto::NotificationLevel) -> &'static str {
    match level {
        dominet_proto::NotificationLevel::Info => "info",
        dominet_proto::NotificationLevel::Warning => "warning",
        dominet_proto::NotificationLevel::Error => "error",
    }
}

/// Pages d'administration : (adresse, cle de traduction du libelle, icone). Reservees aux administrateurs.
const ADMIN_PAGES: [(&str, &str, &str); 20] = [
    ("/admin/locations", tk!("nav-locations"), chrome::MAP_PIN),
    ("/admin/cameras", tk!("nav-cameras"), chrome::VIDEO),
    ("/admin/navbar", tk!("nav-navbar"), chrome::LIST),
    ("/admin/settings", tk!("nav-settings"), chrome::SLIDERS),
    ("/admin/users", tk!("nav-users"), chrome::USERS),
    (
        "/admin/dashboards",
        tk!("nav-dashboards"),
        chrome::DASHBOARD,
    ),
    ("/admin/widgets", tk!("nav-widgets"), chrome::PUZZLE),
    ("/admin/mqtt-sources", tk!("nav-mqtt-sources"), chrome::ZAP),
    ("/admin/devices", tk!("nav-devices-admin"), chrome::CPU),
    (
        "/admin/device-groups",
        tk!("nav-device-groups"),
        chrome::LIST,
    ),
    (
        "/admin/device-labels",
        tk!("nav-device-labels"),
        chrome::THERMOMETER,
    ),
    ("/admin/weather-stations", tk!("nav-weather"), chrome::CHART),
    ("/admin/scenes", tk!("nav-scenes"), chrome::ZAP),
    (
        "/admin/automations",
        tk!("nav-automations"),
        chrome::SLIDERS,
    ),
    ("/admin/alert-rules", tk!("nav-alert-rules"), chrome::ALERT),
    (
        "/admin/prometheus-rules",
        tk!("nav-prometheus-rules"),
        chrome::CHART,
    ),
    (
        "/admin/grafana-rules",
        tk!("nav-grafana-rules"),
        chrome::CHART,
    ),
    ("/admin/logs", tk!("nav-logs"), chrome::LIST),
    ("/admin/mcp", tk!("nav-mcp"), chrome::PUZZLE),
    ("/admin/assistant", tk!("nav-assistant"), chrome::SLIDERS),
];

/// Menu d'administration : tiroir masquable a droite (languette sur le bord de l'ecran). Les liens restent dans le HTML meme
/// ferme (`#vc-nav-admin`), seulement caches : le rendu serveur et les parcours de test les trouvent toujours.
#[component]
fn AdminDrawer() -> impl IntoView {
    let open = RwSignal::new(false);
    // Echap ferme le tiroir (apres l'hydratation seulement : aucun effet cote serveur).
    Effect::new(move |_| {
        let handle = window_event_listener(leptos::ev::keydown, move |ev| {
            if ev.key() == "Escape" {
                open.set(false);
            }
        });
        on_cleanup(move || handle.remove());
    });
    view! {
        <button
            type="button"
            id="vc-admin-toggle"
            class="vc-admin-toggle"
            class:vc-admin-toggle--hidden=move || open.get()
            aria-controls="vc-admin-drawer"
            aria-expanded=move || open.get().to_string()
            title=move || tr!("nav-admin-open")
            aria-label=move || tr!("nav-admin-open")
            on:click=move |ev| {
                ev.stop_propagation();
                open.set(true);
            }
            inner_html=chrome::SLIDERS
        ></button>
        <div class="vc-admin-backdrop" class:vc-admin-backdrop--open=move || open.get() on:click=move |_| open.set(false)></div>
        <aside
            id="vc-admin-drawer"
            class="vc-admin-drawer"
            class:vc-admin-drawer--open=move || open.get()
            aria-hidden=move || (!open.get()).to_string()
        >
            <div class="vc-admin-drawer-head">
                <span class="vc-nav-heading" id="vc-nav-admin-heading">{move_tr!("nav-admin")}</span>
                <button
                    type="button"
                    id="vc-admin-close"
                    class="vc-admin-close"
                    title=move || tr!("nav-admin-close")
                    aria-label=move || tr!("nav-admin-close")
                    on:click=move |_| open.set(false)
                    inner_html=chrome::EXPAND
                ></button>
            </div>
            <nav on:click=move |_| open.set(false)>{admin_links()}</nav>
        </aside>
    }
}

fn admin_links() -> impl IntoView {
    let i18n = expect_context::<I18n>();
    view! {
        <div id="vc-nav-admin">
            {ADMIN_PAGES
                .iter()
                .map(|(href, label, icon)| {
                    view! {
                        <A href=*href attr:class="vc-nav-link">
                            <span class="vc-nav-icon" inner_html=*icon></span>
                            <span>{move || i18n.tr(label)}</span>
                        </A>
                    }
                })
                .collect_view()}
        </div>
    }
}

/// Lien « Alertes » avec la pastille du nombre d'alertes en cours (sujet `alerts`, mise a jour en direct).
#[component]
fn AlertsLink() -> impl IntoView {
    let rt = expect_context::<Realtime>();
    Effect::new(move |_| rt.subscribe(Topic::Alerts));
    on_cleanup(move || rt.unsubscribe(&Topic::Alerts));
    view! {
        <A href="/alerts" attr:class="vc-nav-link" attr:id="vc-nav-alerts">
            <span class="vc-nav-icon" inner_html=chrome::ALERT></span>
            <span>{move_tr!("nav-alerts")}</span>
            <span class="vc-nav-alert-badge" id="vc-alert-badge" hidden=move || rt.alerts.get() == 0>
                {move || rt.alerts.get()}
            </span>
        </A>
    }
}

#[component]
fn Sidebar(
    user: UserDto,
    nav_top: Vec<NavItemDto>,
    nav_bottom: Vec<NavItemDto>,
    collapsed: RwSignal<bool>,
    mobile_open: RwSignal<bool>,
) -> impl IntoView {
    let is_admin = user.role == dominet_proto::RoleDto::Admin;
    let refresh = expect_context::<LayoutRefresh>();
    let state = expect_context::<LayoutState>();
    let navigate = use_navigate();
    let set_collapsed = move |value: bool| {
        collapsed.set(value);
        browser::store_collapsed(value);
    };
    let on_logout = move |_| {
        let navigate = navigate.clone();
        spawn_local(async move {
            let _ = logout().await;
            leave_session(navigate, "/login", state, refresh);
        });
    };

    // Le tiroir d'administration vient AVANT la barre laterale : le style compte sur `.vc-sidebar + .vc-main-wrap` (la marge du contenu),
    // l'`<aside class="vc-sidebar">` doit donc rester le dernier element avant le contenu.
    view! {
        {is_admin.then(|| view! { <AdminDrawer/> })}
        <button
            type="button"
            class="vc-sidebar-open-btn"
            aria-label=move || tr!("nav-open-menu")
            class:vc-sidebar-open-btn--shown=move || collapsed.get()
            on:click=move |_| set_collapsed(false)
            inner_html=chrome::EXPAND
        ></button>
        <button
            type="button"
            class="vc-sidebar-toggle"
            aria-label=move || tr!("nav-menu")
            on:click=move |ev| {
                ev.stop_propagation();
                mobile_open.update(|open| *open = !*open);
            }
            inner_html=chrome::MENU
        ></button>
        <aside
            class="vc-sidebar"
            class:vc-sidebar--collapsed=move || collapsed.get()
            class:vc-sidebar--open=move || mobile_open.get()
        >
            <A href="/" attr:class="vc-brand">
                <span inner_html=chrome::BRAND></span>
                "DomiNet"
            </A>
            <nav class="vc-sidebar-nav" on:click=move |_| mobile_open.set(false)>
                <A href="/" exact=true attr:class="vc-nav-link">
                    <span class="vc-nav-icon" inner_html=chrome::DASHBOARD></span>
                    <span>{move_tr!("nav-dashboard")}</span>
                </A>
                <div id="vc-nav-top">{nav_top.into_iter().map(nav_link).collect_view()}</div>
                <AlertsLink/>
            </nav>
            <SystemGauges/>
            <button
                type="button"
                class="vc-sidebar-collapse-btn"
                title=move || tr!("nav-collapse-title")
                on:click=move |_| set_collapsed(true)
            >
                <span inner_html=chrome::COLLAPSE></span>
                <span>{move_tr!("nav-collapse")}</span>
            </button>
            <div class="vc-sidebar-bottom">
                <button type="button" class="vc-nav-link" on:click=move |_| browser::toggle_fullscreen()>
                    <span class="vc-nav-icon" inner_html=chrome::FULLSCREEN></span>
                    <span>{move_tr!("nav-fullscreen")}</span>
                </button>
                <div id="vc-nav-bottom" on:click=move |_| mobile_open.set(false)>
                    {nav_bottom.into_iter().map(nav_link).collect_view()}
                </div>
                <RealtimeIndicator/>
                <LanguageSwitcher/>
                <A href="/account" attr:class="vc-sidebar-user" attr:title=move || tr!("nav-account")>
                    <span inner_html=chrome::USER></span>
                    <span id="vc-user-name">{user.display_name.clone()}</span>
                </A>
                <button type="button" class="vc-logout-btn" id="vc-logout" on:click=on_logout>
                    <span inner_html=chrome::LOGOUT></span>
                    {move_tr!("nav-logout")}
                </button>
            </div>
        </aside>
    }
}
