// SPDX-License-Identifier: AGPL-3.0-or-later
//! Layout: sidebar (role-based dynamic navigation, collapse, mobile menu, full screen, language,
//! logout) and content area.
//!
//! The content offset depends on no class set by JavaScript: the CSS gets it from the sibling relationship of the
//! elements (`.vc-sidebar + .vc-main-wrap`), so the HTML rendered by the server is already right (no jump at hydration).

use dominet_proto::{LayoutDto, NavItemDto, Topic, UserDto};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_fluent::{I18n, move_tr, tr};
use leptos_router::components::A;
use leptos_router::hooks::use_navigate;

use crate::realtime::{Realtime, RealtimeState};
use crate::server_fns::{layout_data, logout};
use crate::{browser, chrome};

/// Counter to increment to reload the navigation (login, logout, navigation modification).
#[derive(Clone, Copy, Debug)]
pub struct LayoutRefresh(pub RwSignal<u32>);

/// Navigation and current account, shared with the pages (guards, login page) so as to load them only once.
/// The second field is true during a voluntary departure (see [`leave_session`]).
#[derive(Clone, Copy, Debug)]
pub struct LayoutState {
    pub data: Resource<LayoutDto>,
    /// True during a voluntary departure (see [`leave_session`]).
    pub leaving: RwSignal<bool>,
}

/// What the page guards know about the visitor.
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

/// After all the account's sessions are closed (logout, password change): goes to `target`
/// and declares the anonymous state **in the same stride**, before the network reload.
///
/// Without this, the guard of the page being left would redirect to `/login?next=...` (which would overwrite `target`), and the login page,
/// still seeing the account, would send back to the home page.
///
/// The contexts are passed as parameters: after an `await` the reactive owner is no longer guaranteed.
pub fn leave_session(
    navigate: impl Fn(&str, leptos_router::NavigateOptions),
    target: &str,
    state: LayoutState,
    refresh: LayoutRefresh,
) {
    // The guards do not redirect during a voluntary departure (the router's navigation is not instantaneous);
    // the login page resets the flag.
    state.leaving.set(true);
    navigate(target, leptos_router::NavigateOptions::default());
    state.data.set(Some(LayoutDto::default()));
    refresh.0.update(|n| *n += 1);
}

/// True as soon as the interface is hydrated (always false during server rendering). Used to keep the form's
/// submit buttons inactive as long as the browser cannot handle it: otherwise a too-quick submission would go out
/// as an ordinary HTML form (page reloaded, input lost).
pub fn use_hydrated() -> ReadSignal<bool> {
    let (ready, set_ready) = signal(false);
    // Effects only run in the browser, after hydration.
    Effect::new(move |_| set_ready.set(true));
    ready
}

/// Language selector (French / English), without reloading.
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

/// Wraps the pages: sidebar for a logged-in account, content and footer for everyone.
#[component]
pub fn Layout(children: Children) -> impl IntoView {
    let refresh = expect_context::<LayoutRefresh>();
    // *Blocking* resource: the server waits for the response before sending the HTML, so the sidebar is in
    // the initial page (no late insertion that would shift the content).
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
    // Real-time connection: open as long as an account is logged in. The effect restarts at each reload of the
    // data, hence the comparison with the current state (no action if nothing changes).
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
        // Components request or give up subjects: we send the differences to the server.
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
    // After hydration only: restores the remembered collapsed state (no effect on the server side).
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

/// A navigation entry: internal link (browser-side routing) or external (new tab if requested).
fn nav_link(item: NavItemDto) -> impl IntoView {
    let icon = view! { <span class="vc-nav-icon" inner_html=item.icon_svg.clone()></span> };
    // An internal link opened in a new tab is not a router navigation: ordinary HTML link. Likewise for
    // a page served outside the interface (`/docs/`, `/grafana/`): the router does not know it and would display "Page
    // not found"; `rel="external"` tells it to let the browser load the page.
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

/// Real-time connection state (coloured dot + label).
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

/// A gauge: icon, bar and percentage. `pct` is `None` without a measurement (text "…", empty bar).
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
                // The width goes through the CSSOM (allowed by the CSP), never through a rendered `style` attribute.
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

/// Machine gauges (processor, memory, disk, temperature), fed by the `system` subject. As long as no
/// measurement has arrived, or if the connection is cut, the values stay at "…" rather than showing stale data.
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

/// Notifications received in real time, stacked at the bottom right (text only: never HTML).
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

/// Administration pages: (address, label translation key, icon). Reserved to administrators.
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

/// Administration menu: a hideable drawer on the right (tab on the screen edge). The links stay in the HTML even
/// when closed (`#vc-nav-admin`), only hidden: the server render and the test journeys always find them.
#[component]
fn AdminDrawer() -> impl IntoView {
    let open = RwSignal::new(false);
    // Escape closes the drawer (after hydration only: no effect on the server side).
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

/// "Alerts" link with the badge of the number of ongoing alerts (`alerts` subject, updated live).
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

    // The administration drawer comes BEFORE the sidebar: the style relies on `.vc-sidebar + .vc-main-wrap` (the content margin),
    // so the `<aside class="vc-sidebar">` must stay the last element before the content.
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
