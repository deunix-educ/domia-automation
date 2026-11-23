// SPDX-License-Identifier: AGPL-3.0-or-later
//! Navigation administration (sidebar entries): `/admin/navbar`.

use dominet_proto::{NavbarAdminDto, NavbarInput};
use leptos::prelude::*;
use leptos_fluent::move_tr;

use crate::crud::{CrudResult, CrudTable, FieldKind, FieldSpec, Labels, Row, Values};
use crate::guard::RequireAdmin;
use crate::layout::LayoutRefresh;
use crate::server_fns::{delete_navbar, list_navbar, save_navbar};

const TARGETS: &[(&str, &str)] = &[
    ("_self", tk!("navbar-target-self")),
    ("_blank", tk!("navbar-target-blank")),
];
const POSITIONS: &[(&str, &str)] = &[
    ("top", tk!("navbar-position-top")),
    ("bottom", tk!("navbar-position-bottom")),
];

fn row(i: &NavbarAdminDto) -> Row {
    Row {
        id: i.id,
        values: Values::from([
            ("label", i.label.clone()),
            ("icon_name", i.icon_name.clone()),
            ("url", i.url.clone()),
            ("url_target", i.url_target.clone()),
            ("position", i.position.clone()),
            ("sort_order", i.sort_order.to_string()),
            ("admin_only", i.admin_only.to_string()),
        ]),
    }
}

fn text(
    name: &'static str,
    label: &'static str,
    max: u32,
    hint: Option<&'static str>,
) -> FieldSpec {
    FieldSpec {
        name,
        label,
        kind: FieldKind::Text,
        default: "",
        required: true,
        max_len: Some(max),
        hint,
        stored_flag: None,
        create_only: false,
        in_table: true,
    }
}

fn fields() -> Vec<FieldSpec> {
    let select = |name, label, options, in_table| FieldSpec {
        name,
        label,
        kind: FieldKind::Select(options),
        default: "",
        required: true,
        max_len: None,
        hint: None,
        stored_flag: None,
        create_only: false,
        in_table,
    };
    let mut position = select("position", tk!("navbar-position"), POSITIONS, true);
    position.default = "top";
    let mut target = select("url_target", tk!("navbar-target"), TARGETS, false);
    target.default = "_self";
    vec![
        text("label", tk!("navbar-label"), 128, None),
        text(
            "icon_name",
            tk!("navbar-icon"),
            64,
            Some(tk!("navbar-icon-hint")),
        ),
        text("url", tk!("navbar-url"), 255, Some(tk!("navbar-url-hint"))),
        target,
        position,
        FieldSpec {
            name: "sort_order",
            label: tk!("navbar-order"),
            kind: FieldKind::Number {
                min: 0,
                max: i64::from(i16::MAX),
            },
            default: "0",
            required: true,
            max_len: None,
            hint: Some(tk!("navbar-order-hint")),
            stored_flag: None,
            create_only: false,
            in_table: true,
        },
        FieldSpec {
            name: "admin_only",
            label: tk!("navbar-admin-only"),
            kind: FieldKind::Checkbox,
            default: "false",
            required: false,
            max_len: None,
            hint: None,
            stored_flag: None,
            create_only: false,
            in_table: true,
        },
    ]
}

#[component]
pub fn NavbarPage() -> impl IntoView {
    view! {
        <RequireAdmin>
            <main class="vc-container">
                <NavbarAdmin/>
            </main>
        </RequireAdmin>
    }
}

#[component]
fn NavbarAdmin() -> impl IntoView {
    let refresh = expect_context::<LayoutRefresh>();
    let version = RwSignal::new(0u32);
    let list = Resource::new(
        move || version.get(),
        |_| async { list_navbar().await.map_err(|_| ()) },
    );
    let rows = Signal::derive(move || match list.get() {
        Some(Ok(items)) => items.iter().map(row).collect(),
        _ => vec![],
    });

    let save = Action::new(|(id, v): &(Option<i64>, Values)| {
        let get = |k: &str| v.get(k).cloned().unwrap_or_default();
        let input = NavbarInput {
            id: *id,
            icon_name: get("icon_name"),
            label: get("label"),
            url: get("url"),
            url_target: get("url_target"),
            admin_only: get("admin_only") == "true",
            position: get("position"),
            sort_order: get("sort_order"),
        };
        async move {
            save_navbar(input)
                .await
                .map_or(CrudResult::Failed, CrudResult::from)
        }
    });
    let remove = Action::new(|id: &i64| {
        let id = *id;
        async move {
            delete_navbar(id)
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
                    title: tk!("navbar-title"),
                    add: tk!("navbar-add"),
                    edit: tk!("navbar-edit"),
                    empty: tk!("navbar-empty"),
                }
                fields=fields()
                rows
                save
                remove
                // The sidebar displays this navigation: it must follow the modifications.
                on_change=Callback::new(move |()| {
                    version.update(|v| *v += 1);
                    refresh.0.update(|n| *n += 1);
                })
            />
        </Transition>
    }
}
