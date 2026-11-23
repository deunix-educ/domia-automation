// SPDX-License-Identifier: AGPL-3.0-or-later
//! Scenes administration: `/admin/scenes` (R4 lot 2, ADR 0031). A scene has a name, an icon, a location and an
//! ordered list of commands (device + JSON object), published in order on activation.

use dominet_proto::{DeleteOutcome, SaveOutcome, SceneDto, SceneInput};
use leptos::prelude::*;
use leptos_fluent::move_tr;

use crate::crud::{CrudResult, CrudTable, FieldKind, FieldSpec, Labels, Row, Values};
use crate::guard::RequireAdmin;

/// Scenes (active or not), by name (administrators).
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = ListScenesAdmin, prefix = "/sf", endpoint = "scenes-admin/list")]
pub async fn list_scenes_admin() -> Result<Vec<SceneDto>, ServerFnError> {
    use crate::scenes::scene_dtos;
    use crate::server_fns::require_admin;

    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    scene_dtos(&state, false).await
}

/// Creates (`id` absent) or replaces (`id` present) a scene and its commands, in one transaction. Invalid fields are
/// returned one by one.
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = SaveScene, prefix = "/sf", endpoint = "scenes-admin/save", input = server_fn::codec::Json)]
pub async fn save_scene(input: SceneInput) -> Result<SaveOutcome<SceneDto>, ServerFnError> {
    use dominet_domain::command::parse_command;
    use dominet_domain::device::DeviceId;
    use dominet_domain::location::LocationId;
    use dominet_domain::scene::{SceneActionDraft, SceneDraft, SceneId};

    use crate::server_fns::{field_problems, internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let mut draft = SceneDraft::new(input.name.trim());
    draft.icon = input.icon.trim().to_string();
    draft.description = input.description.trim().to_string();
    draft.active = input.active;
    let mut problems = vec![];
    match input.location_id.trim() {
        "" => {}
        text => match text.parse::<i64>() {
            Ok(id) if id > 0 => draft.location = Some(LocationId(id)),
            _ => problems.push(("location_id", dominet_domain::Problem::Invalid)),
        },
    }
    let mut actions_ok = true;
    for (device, command) in &input.actions {
        match (device.trim().parse::<i64>(), parse_command(command)) {
            (Ok(id), Ok(payload)) if id > 0 => draft.actions.push(SceneActionDraft {
                device: DeviceId(id),
                payload,
            }),
            _ => actions_ok = false,
        }
    }
    if !actions_ok {
        problems.push(("actions", dominet_domain::Problem::Invalid));
    }
    // The domain's problems are only consulted if the input could be read (otherwise `actions` would be counted twice).
    for p in draft.problems() {
        if !problems.iter().any(|(f, _)| *f == p.0) {
            problems.push(p);
        }
    }
    if !problems.is_empty() {
        return Ok(SaveOutcome::Invalid(field_problems(problems)));
    }
    // Conflicts (name taken, unknown location or device) are translated into fields by the service shared with the MCP server.
    match dominet_server::config_service::save_scene(&state, input.id.map(SceneId), &draft)
        .await
        .map_err(|e| internal("saving a scene", &e))?
    {
        SaveOutcome::Saved(scene) => {
            tracing::info!(
                admin_id = admin.id.0,
                scene_id = scene.id.0,
                created = input.id.is_none(),
                actions = scene.actions.len(),
                "scene saved"
            );
            let dto = crate::scenes::scene_dtos(&state, false)
                .await?
                .into_iter()
                .find(|s| s.id == scene.id.0);
            Ok(dto.map_or(SaveOutcome::NotFound, SaveOutcome::Saved))
        }
        SaveOutcome::Invalid(p) => Ok(SaveOutcome::Invalid(p)),
        SaveOutcome::NotFound => Ok(SaveOutcome::NotFound),
        SaveOutcome::Blocked(b) => Ok(SaveOutcome::Blocked(b)),
    }
}

/// Deletes a scene; the widgets that targeted it lose their scene, the automations that activate it fail (log).
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = DeleteScene, prefix = "/sf", endpoint = "scenes-admin/delete")]
pub async fn delete_scene(id: i64) -> Result<DeleteOutcome, ServerFnError> {
    use dominet_domain::scene::SceneId;

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let outcome = dominet_server::config_service::delete_scene(&state, SceneId(id))
        .await
        .map_err(|e| internal("deleting a scene", &e))?;
    if outcome == DeleteOutcome::Deleted {
        tracing::info!(admin_id = admin.id.0, scene_id = id, "scene deleted");
    }
    Ok(outcome)
}

fn row(s: &SceneDto) -> Row {
    let pairs: Vec<(String, String)> = s
        .actions
        .iter()
        .map(|a| (a.device_id.to_string(), a.payload.clone()))
        .collect();
    Row {
        id: s.id,
        values: Values::from([
            ("name", s.name.clone()),
            ("icon", s.icon.clone()),
            ("description", s.description.clone()),
            ("location_id", s.location_id.clone()),
            ("active", s.active.to_string()),
            ("actions_count", format!("{}", s.actions.len())),
            (
                "actions",
                serde_json::to_string(&pairs).unwrap_or_else(|_| "[]".to_string()),
            ),
        ]),
    }
}

fn fields() -> Vec<FieldSpec> {
    vec![
        FieldSpec::new("name", tk!("scenes-name"), FieldKind::Text)
            .required()
            .max(64)
            .column(),
        FieldSpec::new("icon", tk!("scenes-icon"), FieldKind::Text)
            .max(32)
            .hint(tk!("scenes-icon-hint"))
            .column(),
        FieldSpec::new("description", tk!("scenes-description"), FieldKind::Text)
            .max(256)
            .column(),
        FieldSpec::new("location_id", tk!("scenes-location"), FieldKind::Choice).column(),
        FieldSpec::new("actions", tk!("scenes-actions"), FieldKind::Pairs)
            .hint(tk!("scenes-actions-hint")),
        FieldSpec::new(
            "actions_count",
            tk!("scenes-actions-count"),
            FieldKind::Display,
        )
        .column(),
        FieldSpec::new("active", tk!("scenes-active"), FieldKind::Checkbox)
            .default_value("true")
            .column(),
    ]
}

#[component]
pub fn ScenesAdminPage() -> impl IntoView {
    view! {
        <RequireAdmin>
            <main class="vc-container">
                <ScenesAdmin/>
            </main>
        </RequireAdmin>
    }
}

#[component]
fn ScenesAdmin() -> impl IntoView {
    crate::devices_admin::provide_choices(&["location_id", "actions"]);
    let version = RwSignal::new(0u32);
    let list = Resource::new(
        move || version.get(),
        |_| async { list_scenes_admin().await.map_err(|_| ()) },
    );
    let rows = Signal::derive(move || match list.get() {
        Some(Ok(items)) => items.iter().map(row).collect(),
        _ => vec![],
    });
    let save = Action::new(|(id, v): &(Option<i64>, Values)| {
        let text = |k: &str| v.get(k).cloned().unwrap_or_default();
        let input = SceneInput {
            id: *id,
            name: text("name"),
            icon: text("icon"),
            description: text("description"),
            location_id: text("location_id"),
            active: v.get("active").is_some_and(|a| a == "true"),
            actions: serde_json::from_str(&text("actions")).unwrap_or_default(),
        };
        async move {
            save_scene(input)
                .await
                .map_or(CrudResult::Failed, CrudResult::from)
        }
    });
    let remove = Action::new(|id: &i64| {
        let id = *id;
        async move {
            delete_scene(id)
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
                    title: tk!("scenes-admin-title"),
                    add: tk!("scenes-add"),
                    edit: tk!("scenes-edit"),
                    empty: tk!("scenes-empty"),
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
