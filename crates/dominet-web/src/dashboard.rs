// SPDX-License-Identifier: AGPL-3.0-or-later
//! Home page: the account's dashboard (R2 lot 4).
//!
//! The dashboards and their widgets come from `dashboard_page` (HTML already sanitised, iframe address already verified). Each
//! widget is placed by **classes** (`vc-c-N`, `vc-r-N`, `vc-w-N`, `vc-h-N`): the CSP forbids the `style` attribute. The
//! device widgets (`device_widget`) subscribe to the `device:<id>` subject, follow the state live and send their commands
//! over MQTT. Cameras are an announced slot (R6).

use dominet_proto::{
    DashboardPageDto, DashboardSummaryDto, DeviceStateDto, ProblemKind, SaveOutcome, Topic,
    WidgetDto,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_fluent::{move_tr, tr};
use leptos_router::NavigateOptions;
use leptos_router::components::A;
use leptos_router::hooks::{use_navigate, use_query_map};
use serde_json::Value;

#[cfg(feature = "hydrate")]
use dominet_proto::MoveOutcome;
use dominet_proto::RoleDto;

use crate::browser;
use crate::chrome;
#[cfg(feature = "hydrate")]
use crate::dashboard_fns::move_widget;
use crate::drag::{self, DragSpec};
use crate::guard::RequireAuth;
use crate::layout::LayoutState;
use crate::live::WhepPlayer;
use crate::realtime::Realtime;
use crate::server_fns::{create_dashboard, dashboard_page};

/// Page `/`: protected, shows the dashboard requested by `?d=<id>`, otherwise the default dashboard.
#[component]
pub fn DashboardPage() -> impl IntoView {
    view! {
        <RequireAuth>
            <DashboardContent/>
        </RequireAuth>
    }
}

/// State shared by editing: counter that restarts the page load, and the last refusal to show.
#[derive(Clone, Copy)]
struct DashboardEdit {
    version: RwSignal<u32>,
    notice: RwSignal<Option<&'static str>>,
}

/// Is the logged-in user an administrator? (reactive)
pub(crate) fn is_admin() -> Memo<bool> {
    let layout = expect_context::<LayoutState>();
    Memo::new(move |_| {
        layout
            .data
            .get()
            .and_then(|d| d.user)
            .is_some_and(|u| u.role == RoleDto::Admin)
    })
}

#[component]
fn DashboardContent() -> impl IntoView {
    let query = use_query_map();
    let requested = move || query.with(|q| q.get("d").and_then(|v| v.parse::<i64>().ok()));
    let edit = DashboardEdit {
        version: RwSignal::new(0),
        notice: RwSignal::new(None),
    };
    provide_context(edit);
    let page = Resource::new(
        move || (requested(), edit.version.get()),
        |(d, _)| async move { dashboard_page(d).await.unwrap_or_default() },
    );
    view! {
        <Transition fallback=|| ()>
            {move || page.get().map(|p| view! { <DashboardView page=p/> })}
        </Transition>
    }
}

#[component]
fn DashboardView(page: DashboardPageDto) -> impl IntoView {
    let Some(current) = page.current.clone() else {
        return view! { <EmptyDashboard/> }.into_any();
    };
    let many = page.dashboards.len() > 1;
    let no_widgets = page.widgets.is_empty();
    let edit = expect_context::<DashboardEdit>();
    let admin = is_admin();
    let edit_link = format!("/admin/widgets?d={}", current.id);
    let picker = many.then(|| {
        let items = page.dashboards.clone();
        let active = current.id;
        view! {
            <details class="vc-dash-picker" id="vc-dash-picker">
                <summary title=move || tr!("dash-picker")>
                    <span id="vc-dash-picker-label">{current.name.clone()}</span>
                    <span inner_html=chrome::CHEVRON_DOWN></span>
                </summary>
                <nav class="vc-dash-picker-menu" aria-label=move || tr!("dash-picker")>
                    {items
                        .into_iter()
                        .map(|d| {
                            let is_active = d.id == active;
                            picker_item(d, is_active)
                        })
                        .collect_view()}
                </nav>
            </details>
        }
    });
    view! {
        <div class="vc-container">
            <div class="vc-page-header">
                <h2>
                    <span inner_html=chrome::DASHBOARD></span>
                    <span id="vc-dash-name">{current.name.clone()}</span>
                </h2>
                <div class="vc-header-actions">
                    {move || {
                        admin
                            .get()
                            .then(|| {
                                view! {
                                    <A href=edit_link.clone() attr:class="vc-btn vc-btn--ghost" attr:id="vc-dash-edit">
                                        <span inner_html=chrome::PLUS></span>
                                        {move_tr!("dash-edit-widgets")}
                                    </A>
                                }
                            })
                    }}
                    {picker}
                </div>
            </div>
            {move || {
                edit.notice
                    .get()
                    .map(|key| {
                        view! {
                            <div class="vc-alert vc-alert--error" role="alert" id="vc-dash-notice">
                                {move || leptos_fluent::I18n::tr(&expect_context::<leptos_fluent::I18n>(), key)}
                            </div>
                        }
                    })
            }}
            {no_widgets
                .then(|| view! { <p class="vc-muted" id="vc-dash-no-widgets">{move_tr!("dash-no-widgets")}</p> })}
        </div>
        <div class="vc-dashboard-grid" id="vc-dashboard-grid" role="region" aria-label=move || tr!("dash-title")>
            <div class="vc-drop-ghost" id="vc-drop-ghost" aria-hidden="true"></div>
            {page.widgets.into_iter().map(|w| view! { <WidgetCard w=w/> }).collect_view()}
        </div>
    }
    .into_any()
}

fn picker_item(d: DashboardSummaryDto, active: bool) -> impl IntoView {
    view! {
        <A
            href=format!("/?d={}", d.id)
            attr:class=if active { "vc-dash-picker-item vc-dash-picker-item--active" } else { "vc-dash-picker-item" }
            attr:aria-current=if active { "true" } else { "false" }
        >
            <span class="vc-dash-check" inner_html=if active { chrome::CHECK } else { "" }></span>
            <span>{d.name}</span>
        </A>
    }
}

/// Screen without a dashboard: invites to create one.
#[component]
fn EmptyDashboard() -> impl IntoView {
    let name = NodeRef::<leptos::html::Input>::new();
    let pending = RwSignal::new(false);
    let problem = RwSignal::new(None::<ProblemKind>);
    let failed = RwSignal::new(false);
    let navigate = use_navigate();
    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        if pending.get_untracked() {
            return;
        }
        let value = browser::input_value(name);
        pending.set(true);
        problem.set(None);
        failed.set(false);
        let navigate = navigate.clone();
        spawn_local(async move {
            match create_dashboard(value).await {
                Ok(SaveOutcome::Saved(d)) => {
                    navigate(&format!("/?d={}", d.id), NavigateOptions::default());
                }
                Ok(SaveOutcome::Invalid(p)) => problem.set(p.first().map(|f| f.kind.clone())),
                _ => failed.set(true),
            }
            pending.set(false);
        });
    };
    view! {
        <div class="vc-dash-empty" id="vc-dash-empty">
            <h2>{move_tr!("dash-empty-title")}</h2>
            <p class="vc-muted">{move_tr!("dash-empty-text")}</p>
            <form on:submit=on_submit>
                <label class="vc-sr-only" for="vc-dash-new-name">{move_tr!("dash-name")}</label>
                <input
                    id="vc-dash-new-name"
                    type="text"
                    maxlength="64"
                    required
                    placeholder=move || tr!("dash-name")
                    node_ref=name
                />
                <button type="submit" id="vc-dash-create" class="vc-btn vc-btn--primary" disabled=move || pending.get()>
                    {move_tr!("dash-create")}
                </button>
            </form>
            {move || {
                problem
                    .get()
                    .map(|kind| {
                        view! {
                            <div class="vc-field-error" id="vc-dash-error">
                                {match kind {
                                    ProblemKind::TooLong { max } => tr!("crud-too-long", {"max" => max}),
                                    ProblemKind::Required => tr!("crud-required"),
                                    _ => tr!("crud-invalid"),
                                }}
                            </div>
                        }
                    })
            }}
            {move || failed.get().then(|| view! { <div class="vc-alert vc-alert--error" role="alert">{move_tr!("error-generic")}</div> })}
        </div>
    }
}

fn type_icon(kind: &str) -> &'static str {
    match kind {
        "camera" => chrome::VIDEO,
        "device" => chrome::CPU,
        "scene" => chrome::ZAP,
        "grafana" => chrome::CHART,
        _ => chrome::DASHBOARD,
    }
}

fn type_label(kind: &str) -> String {
    match kind {
        "camera" => tr!("widget-type-camera"),
        "device" => tr!("widget-type-device"),
        "scene" => tr!("widget-type-scene"),
        "grafana" => tr!("widget-type-grafana"),
        "text" => tr!("widget-type-text"),
        _ => tr!("widget-type-grille"),
    }
}

fn is_on(state: &DeviceStateDto) -> bool {
    state.available && state.state.get("state").and_then(Value::as_str) == Some("ON")
}

/// A widget: frame placed by classes, header and body according to the type.
#[component]
fn WidgetCard(w: WidgetDto) -> impl IntoView {
    let rt = expect_context::<Realtime>();
    let edit = expect_context::<DashboardEdit>();
    let editable = is_admin();
    let spec = DragSpec {
        id: w.id,
        col: w.col,
        row: w.row,
        width: w.width,
        height: w.height,
    };
    let handle = drag::handle();
    let device_id = w.device_id.filter(|_| w.widget_type == "device");
    if let Some(id) = device_id {
        Effect::new(move |_| rt.subscribe(Topic::Device(id)));
        on_cleanup(move || rt.unsubscribe(&Topic::Device(id)));
    }
    let initial = w.device.clone();
    let state = Signal::derive(move || {
        device_id
            .and_then(|id| rt.device(id))
            .or_else(|| initial.clone())
    });
    let classes = format!(
        "vc-widget vc-c-{} vc-r-{} vc-w-{} vc-h-{}",
        (w.col + 1).clamp(1, 12),
        (w.row + 1).clamp(1, 100),
        w.width.clamp(1, 12),
        w.height.clamp(1, 12)
    );
    let kind = w.widget_type.clone();
    let title = w.title.clone();
    let heading = {
        let kind = kind.clone();
        move || {
            if title.is_empty() {
                type_label(&kind)
            } else {
                title.clone()
            }
        }
    };
    let dot = device_id.map(|_| {
        view! {
            <span
                class=move || {
                    if state.get().is_some_and(|s| s.available) {
                        "vc-dev-dot vc-dev-dot--on"
                    } else {
                        "vc-dev-dot vc-dev-dot--off"
                    }
                }
                id=format!("dev-dot-{}", w.id)
            ></span>
        }
    });
    let body = widget_body(&w, state);
    view! {
        <div
            class=classes
            class:vc-widget--on=move || state.get().is_some_and(|s| is_on(&s))
            class:vc-widget--offline=move || state.get().is_some_and(|s| !s.available)
            class:vc-widget--editable=move || editable.get()
            id=format!("widget-{}", w.id)
            data-widget-type=kind.clone()
        >
            <div
                class="vc-widget-header"
                on:pointerdown=move |ev| {
                    #[cfg(feature = "hydrate")]
                    if editable.get_untracked() {
                        edit.notice.set(None);
                        drag::down(handle, &ev, spec);
                    }
                    #[cfg(not(feature = "hydrate"))]
                    let _ = (&ev, edit, handle, spec);
                }
                on:pointermove=move |ev| {
                    #[cfg(feature = "hydrate")]
                    drag::moved(handle, &ev);
                    #[cfg(not(feature = "hydrate"))]
                    let _ = &ev;
                }
                on:pointerup=move |ev| {
                    #[cfg(feature = "hydrate")]
                    if let Some((col, row)) = drag::up(handle, &ev) {
                        spawn_local(async move {
                            let key = match move_widget(spec.id, col, row).await {
                                Ok(MoveOutcome::Moved) => None,
                                Ok(MoveOutcome::Overlap) => Some(tk!("dash-move-overlap")),
                                Ok(MoveOutcome::Invalid) => Some(tk!("dash-move-invalid")),
                                Ok(MoveOutcome::NotFound) => Some(tk!("dash-move-gone")),
                                Err(_) => Some(tk!("dash-move-failed")),
                            };
                            edit.notice.set(key);
                            if key.is_none() || key == Some(tk!("dash-move-gone")) {
                                edit.version.update(|v| *v += 1);
                            }
                        });
                    }
                    #[cfg(not(feature = "hydrate"))]
                    let _ = &ev;
                }
                on:pointercancel=move |_| {
                    #[cfg(feature = "hydrate")]
                    drag::cancel(handle);
                }
            >
                <span class="vc-widget-icon" inner_html=type_icon(&kind)></span>
                <span class="vc-widget-header-title">{heading}</span>
                {dot}
            </div>
            {body}
        </div>
    }
}

fn placeholder(text: impl Fn() -> String + Send + Sync + 'static, id: String) -> AnyView {
    view! {
        <div class="vc-widget-body">
            <div class="vc-widget-placeholder vc-muted" id=id>{text}</div>
        </div>
    }
    .into_any()
}

fn widget_body(w: &WidgetDto, state: Signal<Option<DeviceStateDto>>) -> AnyView {
    match w.widget_type.as_str() {
        "text" => view! {
            <div class="vc-widget-body">
                <div class="vc-widget-text" inner_html=w.content_html.clone()></div>
            </div>
        }
        .into_any(),
        "grille" => view! {
            <div class="vc-widget-body">
                <div class="vc-widget-grid">
                    <div class="vc-widget-grid-cell">"A"</div>
                    <div class="vc-widget-grid-cell">"B"</div>
                    <div class="vc-widget-grid-cell">"C"</div>
                    <div class="vc-widget-grid-cell">"D"</div>
                </div>
            </div>
        }
        .into_any(),
        "grafana" => match &w.iframe_src {
            Some(src) => view! {
                <div class="vc-widget-body vc-widget-body--iframe">
                    <iframe
                        referrerpolicy="no-referrer"
                        sandbox="allow-scripts allow-same-origin"
                        class="vc-widget-iframe"
                        src=src.clone()
                        title=if w.title.is_empty() { "Grafana".to_string() } else { w.title.clone() }
                    ></iframe>
                </div>
            }
            .into_any(),
            None if w.iframe_problem == "forbidden" => {
                placeholder(|| tr!("grafana-forbidden"), format!("grafana-problem-{}", w.id))
            }
            None => placeholder(|| tr!("grafana-not-configured"), format!("grafana-problem-{}", w.id)),
        },
        "camera" => match &w.camera {
            Some(cam) => view! {
                <div class="vc-widget-body vc-widget-body--live">
                    <WhepPlayer camera=cam.clone() instance=u32::try_from(w.id).unwrap_or(0)/>
                </div>
            }
            .into_any(),
            None => placeholder(|| tr!("widget-camera-missing"), format!("widget-camera-missing-{}", w.id)),
        },
        "scene" => crate::scenes::scene_body(w),
        _ => crate::device_widget::device_body(w, state),
    }
}
