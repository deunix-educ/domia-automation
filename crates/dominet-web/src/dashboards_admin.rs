// SPDX-License-Identifier: AGPL-3.0-or-later
//! Administration des tableaux de bord de tous les comptes : `/admin/dashboards`.

use dominet_proto::{DashboardAdminDto, DashboardInput};
use leptos::prelude::*;
use leptos_fluent::move_tr;
use leptos_router::components::A;

use crate::crud::{CrudResult, CrudTable, FieldKind, FieldSpec, Labels, Row, Values};
use crate::dashboard_fns::{delete_dashboard, list_dashboards_admin, save_dashboard};
use crate::guard::RequireAdmin;

fn row(d: &DashboardAdminDto) -> Row {
    Row {
        id: d.id,
        values: Values::from([
            ("name", d.name.clone()),
            ("owner", d.owner.clone()),
            ("is_default", d.is_default.to_string()),
        ]),
    }
}

fn fields() -> Vec<FieldSpec> {
    let base = |name, label, kind| FieldSpec {
        name,
        label,
        kind,
        default: "",
        required: true,
        max_len: None,
        hint: None,
        stored_flag: None,
        create_only: false,
        in_table: true,
    };
    vec![
        FieldSpec {
            max_len: Some(64),
            ..base("name", tk!("dashboards-name"), FieldKind::Text)
        },
        FieldSpec {
            max_len: Some(150),
            create_only: true,
            hint: Some(tk!("dashboards-owner-hint")),
            stored_flag: None,
            ..base("owner", tk!("dashboards-owner"), FieldKind::Text)
        },
        FieldSpec {
            required: false,
            default: "false",
            hint: Some(tk!("dashboards-default-hint")),
            stored_flag: None,
            ..base("is_default", tk!("dashboards-default"), FieldKind::Checkbox)
        },
    ]
}

#[component]
pub fn DashboardsAdminPage() -> impl IntoView {
    view! {
        <RequireAdmin>
            <main class="vc-container">
                <DashboardsAdmin/>
            </main>
        </RequireAdmin>
    }
}

#[component]
fn DashboardsAdmin() -> impl IntoView {
    let version = RwSignal::new(0u32);
    let list = Resource::new(
        move || version.get(),
        |_| async { list_dashboards_admin().await.map_err(|_| ()) },
    );
    let rows = Signal::derive(move || match list.get() {
        Some(Ok(items)) => items.iter().map(row).collect(),
        _ => vec![],
    });
    let save = Action::new(|(id, v): &(Option<i64>, Values)| {
        let get = |k: &str| v.get(k).cloned().unwrap_or_default();
        let input = DashboardInput {
            id: *id,
            name: get("name"),
            owner: get("owner"),
            is_default: get("is_default") == "true",
        };
        async move {
            save_dashboard(input)
                .await
                .map_or(CrudResult::Failed, CrudResult::from)
        }
    });
    let remove = Action::new(|id: &i64| {
        let id = *id;
        async move {
            delete_dashboard(id)
                .await
                .map_or(CrudResult::Failed, CrudResult::from)
        }
    });
    view! {
        <Transition fallback=|| ()>
            {move || {
                matches!(list.get(), Some(Err(()))).then(|| {
                    view! { <div class="vc-alert vc-alert--error" role="alert" id="crud-load-error">{move_tr!("crud-load-error")}</div> }
                })
            }}
            <CrudTable
                labels=Labels {
                    title: tk!("dashboards-title"),
                    add: tk!("dashboards-add"),
                    edit: tk!("dashboards-edit"),
                    empty: tk!("dashboards-empty"),
                }
                fields=fields()
                rows
                save
                remove
                on_change=Callback::new(move |()| version.update(|v| *v += 1))
            />
            <p class="vc-muted">
                <A href="/admin/widgets">{move_tr!("dashboards-widgets-link")}</A>
            </p>
        </Transition>
    }
}
