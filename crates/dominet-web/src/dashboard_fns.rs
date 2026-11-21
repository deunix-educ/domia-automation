// SPDX-License-Identifier: AGPL-3.0-or-later
//! Fonctions serveur d'edition des tableaux de bord (R2 lot 5) : administration des tableaux et des widgets, deplacement
//! d'un widget. **Administrateurs seulement** (comme `is_staff` en reference) : un compte ordinaire voit ses tableaux mais ne
//! les modifie pas ; un administrateur edite ceux de tous les comptes depuis l'administration.

use dominet_proto::{
    DashboardAdminDto, DashboardInput, DeleteOutcome, MoveOutcome, SaveOutcome, WidgetAdminDto,
    WidgetInput, WidgetsPageDto,
};
use leptos::prelude::*;

#[cfg(feature = "ssr")]
use crate::server_fns::{field_problems, internal, require_admin};

#[cfg(feature = "ssr")]
fn widget_admin_dto(w: &dominet_domain::dashboard::Widget) -> WidgetAdminDto {
    WidgetAdminDto {
        id: w.id.0,
        dashboard_id: w.dashboard.0,
        widget_type: w.widget_type.as_str().to_string(),
        title: w.title.clone(),
        presentation: w.presentation.as_str().to_string(),
        device_id: w.device_id.map(|v| v.to_string()).unwrap_or_default(),
        scene_id: w.scene_id.map(|v| v.to_string()).unwrap_or_default(),
        camera_uuid: w.camera_uuid.map(|v| v.to_string()).unwrap_or_default(),
        config: w.config.to_string(),
        content: w.content.clone(),
        col: w.rect.col,
        row: w.rect.row,
        width: w.rect.width,
        height: w.rect.height,
    }
}

/// Tous les tableaux, avec le nom de leur proprietaire.
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = ListDashboardsAdmin, prefix = "/sf", endpoint = "dashboards/list")]
pub async fn list_dashboards_admin() -> Result<Vec<DashboardAdminDto>, ServerFnError> {
    use std::collections::HashMap;

    use dominet_domain::repo::{DashboardRepo, UserRepo};
    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    let names: HashMap<i64, String> = state
        .store
        .users()
        .list()
        .await
        .map_err(|e| internal("account list", &e))?
        .into_iter()
        .map(|u| (u.id.0, u.username))
        .collect();
    let boards = state
        .store
        .dashboards()
        .list_all()
        .await
        .map_err(|e| internal("dashboard list", &e))?;
    Ok(boards
        .into_iter()
        .map(|d| DashboardAdminDto {
            id: d.id.0,
            owner: names.get(&d.owner.0).cloned().unwrap_or_default(),
            name: d.name,
            is_default: d.is_default,
        })
        .collect())
}

/// Cree (`id` absent) ou renomme un tableau. Le proprietaire se designe par son nom d'utilisateur et ne change plus ensuite ;
/// le statut « par defaut » ne peut qu'etre active.
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = SaveDashboard, prefix = "/sf", endpoint = "dashboards/save", input = server_fn::codec::Json)]
pub async fn save_dashboard(
    input: DashboardInput,
) -> Result<SaveOutcome<DashboardAdminDto>, ServerFnError> {
    use dominet_domain::Problem;
    use dominet_domain::dashboard::{DashboardDraft, DashboardId};
    use dominet_domain::repo::UserRepo;

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let draft = DashboardDraft {
        name: input.name.trim().to_string(),
        is_default: input.is_default,
    };
    let mut problems = draft.problems();
    let owner = input.owner.trim();
    let mut owner_id = None;
    let mut owner_name = String::new();
    if input.id.is_none() {
        if owner.is_empty() {
            problems.push(("owner", Problem::Required));
        } else {
            match state
                .store
                .users()
                .find_by_username(owner)
                .await
                .map_err(|e| internal("looking up the owner", &e))?
            {
                Some(u) => {
                    owner_id = Some(u.id);
                    owner_name = u.username;
                }
                None => problems.push(("owner", Problem::Invalid)),
            }
        }
    }
    if !problems.is_empty() {
        return Ok(SaveOutcome::Invalid(field_problems(problems)));
    }
    let saved = dominet_server::config_service::save_dashboard(
        &state,
        input.id.map(DashboardId),
        owner_id,
        &draft,
    )
    .await
    .map_err(|e| internal("saving a dashboard", &e))?;
    let SaveOutcome::Saved(d) = saved else {
        return Ok(match saved {
            SaveOutcome::Invalid(p) => SaveOutcome::Invalid(p),
            SaveOutcome::Blocked(b) => SaveOutcome::Blocked(b),
            _ => SaveOutcome::NotFound,
        });
    };
    if input.id.is_some() {
        owner_name = state
            .store
            .users()
            .find_by_id(d.owner)
            .await
            .map_err(|e| internal("looking up the owner", &e))?
            .map(|u| u.username)
            .unwrap_or_default();
    }
    tracing::info!(
        admin_id = admin.id.0,
        dashboard_id = d.id.0,
        created = input.id.is_none(),
        "dashboard saved"
    );
    Ok(SaveOutcome::Saved(DashboardAdminDto {
        id: d.id.0,
        name: d.name,
        owner: owner_name,
        is_default: d.is_default,
    }))
}

/// Supprime un tableau et ses widgets.
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = DeleteDashboard, prefix = "/sf", endpoint = "dashboards/delete")]
pub async fn delete_dashboard(id: i64) -> Result<DeleteOutcome, ServerFnError> {
    use dominet_domain::dashboard::DashboardId;

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let out = dominet_server::config_service::delete_dashboard(&state, DashboardId(id))
        .await
        .map_err(|e| internal("suppression d'un tableau", &e))?;
    if out == DeleteOutcome::Deleted {
        tracing::info!(
            admin_id = admin.id.0,
            dashboard_id = id,
            "dashboard deleted"
        );
    }
    Ok(out)
}

/// Widgets d'un tableau (n'importe lequel : administration).
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = ListWidgets, prefix = "/sf", endpoint = "widgets/list")]
pub async fn list_widgets(dashboard_id: i64) -> Result<Vec<WidgetAdminDto>, ServerFnError> {
    use dominet_domain::dashboard::DashboardId;
    use dominet_domain::repo::WidgetRepo;
    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    let widgets = state
        .store
        .widgets()
        .list(DashboardId(dashboard_id))
        .await
        .map_err(|e| internal("widget list", &e))?;
    Ok(widgets.iter().map(widget_admin_dto).collect())
}

/// Page d'administration des widgets : le tableau `requested` s'il existe, sinon le premier, et ses widgets. Un seul appel
/// donne le choix et la liste, sans dependance entre deux chargements cote navigateur.
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = WidgetsPage, prefix = "/sf", endpoint = "widgets/page")]
pub async fn widgets_page(requested: Option<i64>) -> Result<WidgetsPageDto, ServerFnError> {
    use dominet_domain::dashboard::DashboardId;
    use dominet_domain::repo::{DashboardRepo, WidgetRepo};
    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    let boards = state
        .store
        .dashboards()
        .list_all()
        .await
        .map_err(|e| internal("dashboard list", &e))?;
    let current = requested
        .filter(|d| boards.iter().any(|b| b.id.0 == *d))
        .or_else(|| boards.first().map(|b| b.id.0));
    let Some(current) = current else {
        return Ok(WidgetsPageDto::default());
    };
    let widgets = state
        .store
        .widgets()
        .list(DashboardId(current))
        .await
        .map_err(|e| internal("widget list", &e))?;
    Ok(WidgetsPageDto {
        current: Some(current),
        widgets: widgets.iter().map(widget_admin_dto).collect(),
    })
}

/// Convertit la saisie en brouillon valide ; les problemes sont rendus champ par champ.
#[cfg(feature = "ssr")]
fn parse_widget(
    input: &WidgetInput,
) -> Result<dominet_domain::dashboard::WidgetDraft, Vec<(&'static str, dominet_domain::Problem)>> {
    use std::str::FromStr;

    use dominet_domain::Problem;
    use dominet_domain::dashboard::{GridRect, Presentation, WidgetDraft, WidgetType};

    let mut bad: Vec<(&'static str, Problem)> = vec![];
    let widget_type = WidgetType::from_str(input.widget_type.trim()).unwrap_or_else(|_| {
        bad.push(("widget_type", Problem::Invalid));
        WidgetType::Text
    });
    let presentation = Presentation::from_str(input.presentation.trim()).unwrap_or_else(|_| {
        bad.push(("presentation", Problem::Invalid));
        Presentation::Auto
    });
    let mut id_field = |name: &'static str, raw: &str| -> Option<i64> {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        raw.parse::<i64>().ok().or_else(|| {
            bad.push((name, Problem::Invalid));
            None
        })
    };
    let device_id = id_field("device_id", &input.device_id);
    let scene_id = id_field("scene_id", &input.scene_id);
    let camera_raw = input.camera_uuid.trim();
    let camera_uuid = if camera_raw.is_empty() {
        None
    } else {
        uuid::Uuid::parse_str(camera_raw).ok().or_else(|| {
            bad.push(("camera_uuid", Problem::Invalid));
            None
        })
    };
    let config_raw = input.config.trim();
    let config = if config_raw.is_empty() {
        serde_json::Value::Object(serde_json::Map::new())
    } else {
        serde_json::from_str(config_raw).unwrap_or_else(|_| {
            bad.push(("config", Problem::Invalid));
            serde_json::Value::Null
        })
    };
    let mut number = |name: &'static str, raw: &str, default: i16| -> i16 {
        let raw = raw.trim();
        if raw.is_empty() {
            return default;
        }
        raw.parse::<i16>().unwrap_or_else(|_| {
            bad.push((name, Problem::Invalid));
            default
        })
    };
    let rect = GridRect {
        col: number("col", &input.col, 0),
        row: number("row", &input.row, 0),
        width: number("width", &input.width, GridRect::DEFAULT.width),
        height: number("height", &input.height, GridRect::DEFAULT.height),
    };
    // Le contenu d'un widget `text` est assaini des l'enregistrement ; pour les autres types il doit rester vide.
    let content = if widget_type == WidgetType::Text {
        dominet_server::html::sanitize_widget_html(&input.content)
    } else {
        input.content.clone()
    };
    let draft = WidgetDraft {
        widget_type,
        title: input.title.trim().to_string(),
        device_id,
        scene_id,
        camera_uuid,
        presentation,
        config,
        content,
        rect,
    };
    let mut problems = draft.problems();
    // Un champ deja signale comme illisible ne doit pas l'etre une seconde fois.
    problems.retain(|(f, _)| bad.iter().all(|(b, _)| b != f));
    bad.extend(problems);
    if bad.is_empty() { Ok(draft) } else { Err(bad) }
}

/// Cree (`id` absent) ou remplace (`id` present) un widget. Un chevauchement est signale sur le champ `col`.
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = SaveWidget, prefix = "/sf", endpoint = "widgets/save", input = server_fn::codec::Json)]
pub async fn save_widget(input: WidgetInput) -> Result<SaveOutcome<WidgetAdminDto>, ServerFnError> {
    use dominet_domain::dashboard::{DashboardId, WidgetId};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let draft = match parse_widget(&input) {
        Ok(d) => d,
        Err(problems) => return Ok(SaveOutcome::Invalid(field_problems(problems))),
    };
    let saved = dominet_server::config_service::save_widget(
        &state,
        input.id.map(WidgetId),
        DashboardId(input.dashboard_id),
        &draft,
    )
    .await
    .map_err(|e| internal("saving a widget", &e))?;
    Ok(match saved {
        SaveOutcome::Saved(w) => {
            tracing::info!(
                admin_id = admin.id.0,
                widget_id = w.id.0,
                created = input.id.is_none(),
                "widget saved"
            );
            SaveOutcome::Saved(widget_admin_dto(&w))
        }
        SaveOutcome::Invalid(p) => SaveOutcome::Invalid(p),
        SaveOutcome::NotFound => SaveOutcome::NotFound,
        SaveOutcome::Blocked(b) => SaveOutcome::Blocked(b),
    })
}

/// Supprime un widget.
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = DeleteWidget, prefix = "/sf", endpoint = "widgets/delete")]
pub async fn delete_widget(id: i64) -> Result<DeleteOutcome, ServerFnError> {
    use dominet_domain::dashboard::WidgetId;

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let out = dominet_server::config_service::delete_widget(&state, WidgetId(id))
        .await
        .map_err(|e| internal("suppression d'un widget", &e))?;
    if out == DeleteOutcome::Deleted {
        tracing::info!(admin_id = admin.id.0, widget_id = id, "widget deleted");
    }
    Ok(out)
}

/// Deplace un widget (position seulement). Refuse hors de la grille ou sur un autre widget : il reste alors ou il etait.
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = MoveWidget, prefix = "/sf", endpoint = "widgets/move")]
pub async fn move_widget(id: i64, col: i16, row: i16) -> Result<MoveOutcome, ServerFnError> {
    use dominet_domain::RepoError;
    use dominet_domain::dashboard::{GridRect, WidgetId};
    use dominet_domain::repo::WidgetRepo;

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let repo = state.store.widgets();
    let Some(current) = repo
        .get(WidgetId(id))
        .await
        .map_err(|e| internal("reading a widget", &e))?
    else {
        return Ok(MoveOutcome::NotFound);
    };
    let target = GridRect {
        col,
        row,
        ..current.rect
    };
    if !target.problems().is_empty() {
        return Ok(MoveOutcome::Invalid);
    }
    match repo.move_to(WidgetId(id), col, row).await {
        Ok(_) => {
            tracing::info!(
                admin_id = admin.id.0,
                widget_id = id,
                col,
                row,
                "widget moved"
            );
            Ok(MoveOutcome::Moved)
        }
        Err(RepoError::Conflict { .. }) => Ok(MoveOutcome::Overlap),
        Err(RepoError::NotFound) => Ok(MoveOutcome::NotFound),
        Err(e) => Err(internal("moving a widget", &e)),
    }
}
