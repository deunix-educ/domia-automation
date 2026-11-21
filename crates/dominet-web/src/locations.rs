// SPDX-License-Identifier: AGPL-3.0-or-later
//! Administration des emplacements (pieces, zones) : `/admin/locations`.

use dominet_proto::{LocationDto, LocationInput};
use leptos::prelude::*;
use leptos_fluent::move_tr;

use crate::crud::{CrudResult, CrudTable, FieldKind, FieldSpec, Labels, Row, Values};
use crate::guard::RequireAdmin;
use crate::server_fns::{delete_location, list_locations, save_location};

fn row(l: &LocationDto) -> Row {
    Row {
        id: l.id,
        values: Values::from([
            ("name", l.name.clone()),
            ("description", l.description.clone()),
            ("active", l.active.to_string()),
        ]),
    }
}

fn fields() -> Vec<FieldSpec> {
    vec![
        FieldSpec {
            name: "name",
            label: tk!("locations-name"),
            kind: FieldKind::Text,
            default: "",
            required: true,
            max_len: Some(64),
            hint: None,
            stored_flag: None,
            create_only: false,
            in_table: true,
        },
        FieldSpec {
            name: "description",
            label: tk!("locations-description"),
            kind: FieldKind::Text,
            default: "",
            required: false,
            max_len: Some(128),
            hint: None,
            stored_flag: None,
            create_only: false,
            in_table: true,
        },
        FieldSpec {
            name: "active",
            label: tk!("locations-active"),
            kind: FieldKind::Checkbox,
            default: "true",
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
pub fn LocationsPage() -> impl IntoView {
    view! {
        <RequireAdmin>
            <main class="vc-container">
                <LocationsAdmin/>
            </main>
        </RequireAdmin>
    }
}

#[component]
fn LocationsAdmin() -> impl IntoView {
    let version = RwSignal::new(0u32);
    let list = Resource::new(
        move || version.get(),
        |_| async { list_locations().await.map_err(|_| ()) },
    );
    let rows = Signal::derive(move || match list.get() {
        Some(Ok(items)) => items.iter().map(row).collect(),
        _ => vec![],
    });

    let save = Action::new(|(id, v): &(Option<i64>, Values)| {
        let input = LocationInput {
            id: *id,
            name: v.get("name").cloned().unwrap_or_default(),
            description: v.get("description").cloned().unwrap_or_default(),
            active: v.get("active").is_some_and(|a| a == "true"),
        };
        async move {
            save_location(input)
                .await
                .map_or(CrudResult::Failed, CrudResult::from)
        }
    });
    let remove = Action::new(|id: &i64| {
        let id = *id;
        async move {
            delete_location(id)
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
                    title: tk!("locations-title"),
                    add: tk!("locations-add"),
                    edit: tk!("locations-edit"),
                    empty: tk!("locations-empty"),
                }
                fields=fields()
                rows
                save
                remove
                on_change=Callback::new(move |()| version.update(|v| *v += 1))
            />
        </Transition>
    }
}
