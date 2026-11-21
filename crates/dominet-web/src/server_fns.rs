// SPDX-License-Identifier: AGPL-3.0-or-later
//! Fonctions serveur : declarees ici pour que le navigateur dispose du stub client, executees cote serveur
//! avec l'etat de l'application (fourni par le contexte Leptos) et la session de la requete.

use dominet_proto::{
    BroadcastOutcome, ChangePasswordOutcome, DashboardPageDto, DashboardSummaryDto, DeleteOutcome,
    InjectOutcome, LayoutDto, LocationDto, LocationInput, LoginOutcome, NavbarAdminDto,
    NavbarInput, SaveOutcome, SettingDto, SettingOutcome, UserAdminDto, UserDto, UserInput,
};
use leptos::prelude::*;

/// Administrateur connecte, ou refus (403). Chaque fonction d'administration commence par la : les gardes de pages
/// ne servent qu'a l'affichage.
#[cfg(feature = "ssr")]
pub(crate) async fn require_admin(
    state: &dominet_server::AppState,
) -> Result<dominet_domain::user::User, ServerFnError> {
    use dominet_server::auth::extract::RequireAdmin;
    if let Ok(RequireAdmin(user)) =
        leptos_axum::extract_with_state::<RequireAdmin, dominet_server::AppState>(state).await
    {
        Ok(user)
    } else {
        expect_context::<leptos_axum::ResponseOptions>()
            .set_status(axum::http::StatusCode::FORBIDDEN);
        Err(ServerFnError::new("access denied"))
    }
}

/// Compte connecte (n'importe quel role), ou refus (401).
#[cfg(feature = "ssr")]
pub(crate) async fn require_user(
    state: &dominet_server::AppState,
) -> Result<dominet_domain::user::User, ServerFnError> {
    use dominet_server::auth::extract::CurrentUser;
    if let Ok(CurrentUser(user)) =
        leptos_axum::extract_with_state::<CurrentUser, dominet_server::AppState>(state).await
    {
        Ok(user)
    } else {
        expect_context::<leptos_axum::ResponseOptions>()
            .set_status(axum::http::StatusCode::UNAUTHORIZED);
        Err(ServerFnError::new("authentication required"))
    }
}

/// Erreur interne : le detail est journalise, jamais renvoye au navigateur.
#[cfg(feature = "ssr")]
pub(crate) fn internal(context: &str, err: &dyn std::fmt::Display) -> ServerFnError {
    tracing::error!(%err, "{context}");
    ServerFnError::new("internal error")
}

/// Compte connecte, ou `None` si la session est absente, expiree ou le compte desactive.
///
/// # Errors
/// Jamais cote serveur (un refus d'authentification vaut `None`) ; erreur de transport cote navigateur.
#[server(name = Whoami, prefix = "/sf", endpoint = "whoami")]
pub async fn whoami() -> Result<Option<UserDto>, ServerFnError> {
    use dominet_server::AppState;
    use dominet_server::auth::extract::CurrentUser;

    let state = expect_context::<AppState>();
    Ok(
        leptos_axum::extract_with_state::<CurrentUser, AppState>(&state)
            .await
            .ok()
            .map(|CurrentUser(user)| dominet_server::web::user_dto(&user)),
    )
}

/// Compte connecte et navigation visible pour son role (vide pour un visiteur anonyme).
///
/// # Errors
/// Erreur si la base de donnees est injoignable.
#[server(name = LayoutData, prefix = "/sf", endpoint = "layout")]
pub async fn layout_data() -> Result<LayoutDto, ServerFnError> {
    use dominet_domain::navbar::{NavPosition, NavbarItem};
    use dominet_domain::repo::NavbarRepo;
    use dominet_proto::NavItemDto;
    use dominet_server::AppState;
    use dominet_server::auth::extract::CurrentUser;

    let state = expect_context::<AppState>();
    let Ok(CurrentUser(user)) =
        leptos_axum::extract_with_state::<CurrentUser, AppState>(&state).await
    else {
        return Ok(LayoutDto::default());
    };
    let items = state
        .store
        .navbar()
        .list()
        .await
        .map_err(|e| internal("reading the navigation", &e))?;
    let dto = |i: &NavbarItem| NavItemDto {
        label: i.label.clone(),
        url: i.url.clone(),
        url_target: i.url_target.clone(),
        icon_svg: crate::icons::svg(&i.icon_name, 16),
    };
    let visible = items.iter().filter(|i| i.visible_to(user.role));
    let (top, bottom): (Vec<_>, Vec<_>) = visible.partition(|i| i.position == NavPosition::Top);
    Ok(LayoutDto {
        user: Some(dominet_server::web::user_dto(&user)),
        nav_top: top.into_iter().map(dto).collect(),
        nav_bottom: bottom.into_iter().map(dto).collect(),
    })
}

/// Ferme la session courante (idempotent : sans session, ne fait rien).
///
/// # Errors
/// Erreur si le store de sessions echoue.
#[server(name = Logout, prefix = "/sf", endpoint = "logout")]
pub async fn logout() -> Result<(), ServerFnError> {
    let session: tower_sessions::Session = leptos_axum::extract().await?;
    dominet_server::auth::session::close(&session)
        .await
        .map_err(|e| internal("signing out", &e))
}

/// Tentative de connexion. Les refus attendus (identifiants faux, compte bloque) sont des valeurs du resultat ;
/// seule une panne (base, store de sessions) est une erreur.
///
/// # Errors
/// Erreur interne si la base ou le store de sessions echoue.
#[server(name = Login, prefix = "/sf", endpoint = "login")]
pub async fn login(username: String, password: String) -> Result<LoginOutcome, ServerFnError> {
    use dominet_server::AppState;
    use dominet_server::auth::client_ip::ClientIp;
    use dominet_server::auth::service::AuthError;

    let state = expect_context::<AppState>();
    let ClientIp(ip) = leptos_axum::extract_with_state::<ClientIp, AppState>(&state)
        .await
        .map_err(|_| ServerFnError::new("client address not found"))?;
    let session: tower_sessions::Session = leptos_axum::extract().await?;
    match state.auth.authenticate(&username, &password, &ip).await {
        Ok(user) => {
            dominet_server::auth::session::open(&session, &user)
                .await
                .map_err(|e| internal("signing in", &e))?;
            tracing::info!(user_id = user.id.0, %ip, "login succeeded");
            Ok(LoginOutcome::Success(dominet_server::web::user_dto(&user)))
        }
        Err(AuthError::InvalidCredentials) => Ok(LoginOutcome::InvalidCredentials),
        Err(AuthError::Locked { retry_after_s }) => Ok(LoginOutcome::Locked { retry_after_s }),
        Err(e) => Err(internal("connection", &e)),
    }
}

/// Change le mot de passe du compte connecte. En cas de succes, **toutes** ses sessions sont fermees (y compris
/// celle-ci) : l'interface renvoie vers la page de connexion.
///
/// # Errors
/// Refus d'authentification si personne n'est connecte ; erreur interne si la base echoue.
#[server(name = ChangePassword, prefix = "/sf", endpoint = "password")]
pub async fn change_password(
    current_password: String,
    new_password: String,
) -> Result<ChangePasswordOutcome, ServerFnError> {
    use dominet_server::AppState;
    use dominet_server::auth::client_ip::ClientIp;
    use dominet_server::auth::extract::CurrentUser;
    use dominet_server::auth::service::AuthError;

    let state = expect_context::<AppState>();
    let CurrentUser(user) = leptos_axum::extract_with_state::<CurrentUser, AppState>(&state)
        .await
        .map_err(|_| ServerFnError::new("not authenticated"))?;
    let ClientIp(ip) = leptos_axum::extract_with_state::<ClientIp, AppState>(&state)
        .await
        .map_err(|_| ServerFnError::new("client address not found"))?;
    match state
        .auth
        .change_own_password(&user, &current_password, &new_password, &ip)
        .await
    {
        Ok(()) => {
            let session: tower_sessions::Session = leptos_axum::extract().await?;
            dominet_server::auth::session::close(&session)
                .await
                .map_err(|e| internal("signing out", &e))?;
            tracing::info!(user_id = user.id.0, "password changed, sessions revoked");
            Ok(ChangePasswordOutcome::Changed)
        }
        Err(AuthError::InvalidCredentials) => Ok(ChangePasswordOutcome::WrongCurrent),
        Err(AuthError::Locked { retry_after_s }) => {
            Ok(ChangePasswordOutcome::Locked { retry_after_s })
        }
        Err(AuthError::Invalid(_)) => match password_problem(&new_password, &user.username) {
            Some(problem) => Ok(ChangePasswordOutcome::Rejected(problem)),
            None => Err(internal("password change", &"refusal without reason")),
        },
        Err(e) => Err(internal("password change", &e)),
    }
}

/// Motif de refus d'un mot de passe par la politique, ou `None` s'il est accepte.
#[cfg(feature = "ssr")]
fn password_problem(password: &str, username: &str) -> Option<dominet_proto::PasswordProblem> {
    use dominet_domain::auth::{PasswordPolicy, PasswordViolation};
    use dominet_proto::PasswordProblem;
    PasswordPolicy::violation(password, username).map(|v| match v {
        PasswordViolation::TooShort => PasswordProblem::TooShort {
            min: u32::try_from(PasswordPolicy::MIN_LEN).unwrap_or(10),
        },
        PasswordViolation::TooLong => PasswordProblem::TooLong {
            max: u32::try_from(PasswordPolicy::MAX_LEN).unwrap_or(128),
        },
        PasswordViolation::AllDigits => PasswordProblem::AllDigits,
        PasswordViolation::SameAsUsername => PasswordProblem::SameAsUsername,
    })
}

/// Problemes du domaine -> problemes de formulaire (champ par champ).
#[cfg(feature = "ssr")]
pub(crate) fn field_problems(
    problems: Vec<(&'static str, dominet_domain::Problem)>,
) -> Vec<dominet_proto::FieldProblem> {
    use dominet_domain::Problem;
    use dominet_proto::{FieldProblem, ProblemKind};
    problems
        .into_iter()
        .map(|(field, p)| FieldProblem {
            field: field.to_string(),
            kind: match p {
                Problem::Required => ProblemKind::Required,
                Problem::TooLong { max } => ProblemKind::TooLong {
                    max: u32::try_from(max).unwrap_or(u32::MAX),
                },
                Problem::Invalid => ProblemKind::Invalid,
                Problem::OutOfRange { min, max } => ProblemKind::OutOfRange { min, max },
            },
        })
        .collect()
}

#[cfg(feature = "ssr")]
fn location_dto(l: &dominet_domain::location::Location) -> LocationDto {
    LocationDto {
        id: l.id.0,
        name: l.name.clone(),
        description: l.description.clone(),
        active: l.active,
    }
}

/// Emplacements, tries par nom (administrateurs).
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = ListLocations, prefix = "/sf", endpoint = "locations/list")]
pub async fn list_locations() -> Result<Vec<LocationDto>, ServerFnError> {
    use dominet_domain::repo::LocationRepo;

    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    let items = state
        .store
        .locations()
        .list()
        .await
        .map_err(|e| internal("place list", &e))?;
    Ok(items.iter().map(location_dto).collect())
}

/// Cree (`id` absent) ou remplace (`id` present) un emplacement. Les champs invalides sont renvoyes un par un.
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = SaveLocation, prefix = "/sf", endpoint = "locations/save", input = server_fn::codec::Json)]
pub async fn save_location(
    input: LocationInput,
) -> Result<SaveOutcome<LocationDto>, ServerFnError> {
    use dominet_domain::location::{LocationDraft, LocationId};
    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let draft = LocationDraft {
        name: input.name.trim().to_string(),
        description: input.description.trim().to_string(),
        active: input.active,
    };
    match dominet_server::config_service::save_location(&state, input.id.map(LocationId), &draft)
        .await
        .map_err(|e| internal("saving a place", &e))?
    {
        SaveOutcome::Saved(l) => {
            tracing::info!(
                admin_id = admin.id.0,
                location_id = l.id.0,
                created = input.id.is_none(),
                "place saved"
            );
            Ok(SaveOutcome::Saved(location_dto(&l)))
        }
        SaveOutcome::Invalid(p) => Ok(SaveOutcome::Invalid(p)),
        SaveOutcome::NotFound => Ok(SaveOutcome::NotFound),
        SaveOutcome::Blocked(b) => Ok(SaveOutcome::Blocked(b)),
    }
}

/// Supprime un emplacement.
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = DeleteLocation, prefix = "/sf", endpoint = "locations/delete")]
pub async fn delete_location(id: i64) -> Result<DeleteOutcome, ServerFnError> {
    use dominet_domain::location::LocationId;

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let outcome = dominet_server::config_service::delete_location(&state, LocationId(id))
        .await
        .map_err(|e| internal("suppression d'un emplacement", &e))?;
    if outcome == DeleteOutcome::Deleted {
        tracing::info!(admin_id = admin.id.0, location_id = id, "place deleted");
    }
    Ok(outcome)
}

#[cfg(feature = "ssr")]
fn navbar_admin_dto(i: &dominet_domain::navbar::NavbarItem) -> NavbarAdminDto {
    NavbarAdminDto {
        id: i.id.0,
        icon_name: i.icon_name.clone(),
        label: i.label.clone(),
        url: i.url.clone(),
        url_target: i.url_target.clone(),
        admin_only: i.admin_only,
        position: i.position.as_str().to_string(),
        sort_order: i32::from(i.sort_order),
    }
}

/// Entrees de navigation, dans l'ordre d'affichage : haut puis bas (administrateurs).
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = ListNavbar, prefix = "/sf", endpoint = "navbar/list")]
pub async fn list_navbar() -> Result<Vec<NavbarAdminDto>, ServerFnError> {
    use dominet_domain::repo::NavbarRepo;

    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    let items = state
        .store
        .navbar()
        .list()
        .await
        .map_err(|e| internal("navigation list", &e))?;
    Ok(items.iter().map(navbar_admin_dto).collect())
}

/// Cree (`id` absent) ou remplace (`id` present) une entree de navigation. Les champs invalides sont renvoyes un
/// par un ; un nom d'icone inexistant dans Lucide est refuse (sinon l'entree afficherait une icone de repli).
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = SaveNavbar, prefix = "/sf", endpoint = "navbar/save", input = server_fn::codec::Json)]
pub async fn save_navbar(input: NavbarInput) -> Result<SaveOutcome<NavbarAdminDto>, ServerFnError> {
    use std::str::FromStr;

    use dominet_domain::Problem;
    use dominet_domain::navbar::{NavPosition, NavbarDraft, NavbarItemId};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;

    let mut problems: Vec<(&'static str, Problem)> = vec![];
    let position = NavPosition::from_str(input.position.trim()).unwrap_or_else(|_| {
        problems.push(("position", Problem::Invalid));
        NavPosition::Top
    });
    let range = Problem::OutOfRange {
        min: 0,
        max: i64::from(i16::MAX),
    };
    let raw_order = input.sort_order.trim();
    let sort_order = if raw_order.is_empty() {
        0
    } else if let Ok(n) = raw_order.parse::<i64>()
        && let Ok(n) = i16::try_from(n)
    {
        n
    } else {
        problems.push(("sort_order", range));
        0
    };
    let draft = NavbarDraft {
        icon_name: input.icon_name.trim().to_string(),
        label: input.label.trim().to_string(),
        url: input.url.trim().to_string(),
        url_target: input.url_target.trim().to_string(),
        admin_only: input.admin_only,
        position,
        sort_order,
    };
    if !problems.is_empty() {
        // The form itself is wrong: say that first, with what the draft would add (an unknown Lucide icon is checked by the
        // shared service, which the interface and the MCP server both use).
        problems.extend(draft.problems());
        return Ok(SaveOutcome::Invalid(field_problems(problems)));
    }

    match dominet_server::config_service::save_navbar(&state, input.id.map(NavbarItemId), &draft)
        .await
    {
        Ok(SaveOutcome::Saved(item)) => {
            tracing::info!(
                admin_id = admin.id.0,
                navbar_id = item.id.0,
                created = input.id.is_none(),
                "navigation saved"
            );
            Ok(SaveOutcome::Saved(navbar_admin_dto(&item)))
        }
        Ok(other) => Ok(other.map(|i| navbar_admin_dto(&i))),
        Err(e) => Err(internal("saving the navigation", &e)),
    }
}

/// Supprime une entree de navigation.
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = DeleteNavbar, prefix = "/sf", endpoint = "navbar/delete")]
pub async fn delete_navbar(id: i64) -> Result<DeleteOutcome, ServerFnError> {
    use dominet_domain::navbar::NavbarItemId;

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    match dominet_server::config_service::delete_navbar(&state, NavbarItemId(id)).await {
        Ok(outcome) => {
            if matches!(outcome, DeleteOutcome::Deleted) {
                tracing::info!(admin_id = admin.id.0, navbar_id = id, "navigation deleted");
            }
            Ok(outcome)
        }
        Err(e) => Err(internal("deleting the navigation", &e)),
    }
}

#[cfg(feature = "ssr")]
fn setting_dto(v: dominet_server::settings::SettingView) -> SettingDto {
    SettingDto {
        key: v.key.to_string(),
        value_type: v.value_type.to_string(),
        category: v.category.to_string(),
        description: v.description.to_string(),
        is_secret: v.is_secret,
        value: v.value,
        is_set: v.is_set,
        from_default: v.from_default,
        readable: v.readable,
        updated_at: v.updated_at.map(dominet_server::timefmt::rfc3339),
    }
}

/// Les 17 reglages applicatifs, tries par categorie ; la valeur d'un secret n'est jamais renvoyee (administrateurs).
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = ListSettings, prefix = "/sf", endpoint = "settings/list")]
pub async fn list_settings() -> Result<Vec<SettingDto>, ServerFnError> {
    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    let views = state
        .settings
        .list()
        .await
        .map_err(|e| internal("settings list", &e))?;
    Ok(views.into_iter().map(setting_dto).collect())
}

/// Modifie un reglage (valide selon son type, chiffre s'il est secret). La valeur n'est jamais journalisee.
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = SaveSetting, prefix = "/sf", endpoint = "settings/save")]
pub async fn save_setting(key: String, value: String) -> Result<SettingOutcome, ServerFnError> {
    use dominet_server::settings::SettingsError;

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    match state.settings.set(&key, &value).await {
        Ok(view) => {
            tracing::info!(admin_id = admin.id.0, %key, secret = view.is_secret, "setting changed");
            Ok(SettingOutcome::Saved(setting_dto(view)))
        }
        Err(SettingsError::Invalid(_)) => Ok(SettingOutcome::Invalid),
        Err(SettingsError::MasterKeyMissing) => Ok(SettingOutcome::MasterKeyMissing),
        Err(SettingsError::UnknownKey(_)) => Ok(SettingOutcome::UnknownKey),
        Err(e) => Err(internal("saving a setting", &e)),
    }
}

#[cfg(feature = "ssr")]
fn user_admin_dto(u: &dominet_domain::user::User) -> UserAdminDto {
    UserAdminDto {
        id: u.id.0,
        username: u.username.clone(),
        first_name: u.first_name.clone(),
        last_name: u.last_name.clone(),
        email: u.email.clone(),
        mobile: u.mobile.clone(),
        role: if u.role.is_admin() {
            dominet_proto::RoleDto::Admin
        } else {
            dominet_proto::RoleDto::User
        },
        is_active: u.is_active,
        last_login_at: u.last_login_at.map(dominet_server::timefmt::rfc3339),
    }
}

/// Valide les champs d'un compte saisi dans l'administration : role reconnu, identifiant (creation seulement), champs
/// facultatifs, mot de passe (obligatoire a la creation, sinon seulement s'il est fourni). Renvoie aussi le role lu.
#[cfg(feature = "ssr")]
fn user_input_problems(
    input: &UserInput,
    username: &str,
    creating: bool,
) -> (dominet_domain::user::Role, Vec<dominet_proto::FieldProblem>) {
    use std::str::FromStr;

    use dominet_domain::Problem;
    use dominet_domain::user::{AccountFields, Role, Username};
    use dominet_proto::{FieldProblem, ProblemKind};

    let mut domain: Vec<(&'static str, Problem)> = vec![];
    let role = Role::from_str(input.role.trim()).unwrap_or_else(|_| {
        domain.push(("role", Problem::Invalid));
        Role::User
    });
    if creating && let Some(p) = Username::problem(username) {
        domain.push(("username", p));
    }
    domain.extend(
        AccountFields {
            email: &input.email,
            first_name: &input.first_name,
            last_name: &input.last_name,
            mobile: &input.mobile,
        }
        .problems(),
    );
    let mut problems = field_problems(domain);
    let kind = if input.password.is_empty() {
        creating.then_some(ProblemKind::Required)
    } else {
        password_problem(&input.password, username).map(ProblemKind::Password)
    };
    if let Some(kind) = kind {
        problems.push(FieldProblem {
            field: "password".into(),
            kind,
        });
    }
    (role, problems)
}

/// Comptes, tries par identifiant (administrateurs). Jamais de condensat ni de mot de passe.
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = ListUsers, prefix = "/sf", endpoint = "users/list")]
pub async fn list_users() -> Result<Vec<UserAdminDto>, ServerFnError> {
    use dominet_domain::repo::UserRepo;

    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    let mut users = state
        .store
        .users()
        .list()
        .await
        .map_err(|e| internal("account list", &e))?;
    users.sort_by_key(|u| u.username.to_lowercase());
    Ok(users.iter().map(user_admin_dto).collect())
}

/// Cree (`id` absent) ou modifie un compte. Une modification peut reinitialiser le mot de passe (ce qui ferme toutes les
/// sessions du compte). Refus : retirer le dernier administrateur actif ; pour soi-meme, se desactiver, se retrograder
/// ou changer son mot de passe ici.
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = SaveUser, prefix = "/sf", endpoint = "users/save", input = server_fn::codec::Json)]
pub async fn save_user(input: UserInput) -> Result<SaveOutcome<UserAdminDto>, ServerFnError> {
    use dominet_domain::RepoError;
    use dominet_domain::repo::UserRepo;
    use dominet_domain::user::{UserId, UserProfile};
    use dominet_proto::{Blocked, FieldProblem, ProblemKind};
    use dominet_server::auth::service::{AuthError, NewAccount};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let users = state.store.users();

    // Compte vise (modification) : l'identifiant est immuable, on garde celui de la base.
    let existing = match input.id {
        Some(id) => match users.find_by_id(UserId(id)).await {
            Ok(Some(u)) => Some(u),
            Ok(None) => return Ok(SaveOutcome::NotFound),
            Err(e) => return Err(internal("reading an account", &e)),
        },
        None => None,
    };
    let username = existing
        .as_ref()
        .map_or_else(|| input.username.trim().to_string(), |u| u.username.clone());

    let (role, problems) = user_input_problems(&input, &username, existing.is_none());
    let new_password = (!input.password.is_empty()).then_some(input.password.as_str());
    if !problems.is_empty() {
        return Ok(SaveOutcome::Invalid(problems));
    }

    // Garde-fous sur son propre compte (la session serait coupee en pleine page).
    if let Some(target) = &existing
        && target.id == admin.id
    {
        if new_password.is_some() {
            return Ok(SaveOutcome::Blocked(Blocked::OwnPassword));
        }
        if !role.is_admin() || !input.is_active {
            return Ok(SaveOutcome::Blocked(Blocked::Yourself));
        }
    }

    let saved = match &existing {
        None => {
            state
                .auth
                .create_account(&NewAccount {
                    username: &username,
                    email: &input.email,
                    first_name: &input.first_name,
                    last_name: &input.last_name,
                    mobile: &input.mobile,
                    role,
                    is_active: input.is_active,
                    password: &input.password,
                })
                .await
        }
        Some(target) => {
            let profile = UserProfile {
                email: input.email.clone(),
                first_name: input.first_name.clone(),
                last_name: input.last_name.clone(),
                mobile: input.mobile.clone(),
                role,
                is_active: input.is_active,
            };
            state
                .auth
                .update_account(target.id, &profile, new_password)
                .await
        }
    };
    match saved {
        Ok(user) => {
            // Jamais le mot de passe : seulement qui a fait quoi.
            tracing::info!(
                admin_id = admin.id.0,
                user_id = user.id.0,
                created = existing.is_none(),
                password_reset = new_password.is_some() && existing.is_some(),
                "account saved"
            );
            Ok(SaveOutcome::Saved(user_admin_dto(&user)))
        }
        Err(AuthError::Repo(RepoError::Conflict { .. })) => {
            Ok(SaveOutcome::Invalid(vec![FieldProblem {
                field: "username".into(),
                kind: ProblemKind::Taken,
            }]))
        }
        Err(AuthError::Repo(RepoError::LastAdmin)) => Ok(SaveOutcome::Blocked(Blocked::LastAdmin)),
        Err(AuthError::Repo(RepoError::NotFound)) => Ok(SaveOutcome::NotFound),
        Err(e) => Err(internal("saving an account", &e)),
    }
}

/// Supprime un compte et toutes ses sessions. Refus : soi-meme, le dernier administrateur actif.
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = DeleteUser, prefix = "/sf", endpoint = "users/delete")]
pub async fn delete_user(id: i64) -> Result<DeleteOutcome, ServerFnError> {
    use dominet_domain::RepoError;
    use dominet_domain::repo::UserRepo;
    use dominet_domain::user::UserId;
    use dominet_proto::Blocked;

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    if admin.id.0 == id {
        return Ok(DeleteOutcome::Blocked(Blocked::Yourself));
    }
    match state.store.users().delete(UserId(id)).await {
        Ok(()) => {
            tracing::info!(admin_id = admin.id.0, user_id = id, "account deleted");
            Ok(DeleteOutcome::Deleted)
        }
        Err(RepoError::NotFound) => Ok(DeleteOutcome::NotFound),
        Err(RepoError::LastAdmin) => Ok(DeleteOutcome::Blocked(Blocked::LastAdmin)),
        Err(RepoError::Conflict { .. }) => Ok(DeleteOutcome::InUse),
        Err(e) => Err(internal("deleting an account", &e)),
    }
}

/// Diffuse un message a toutes les connexions temps reel ouvertes (administrateurs). Sert aussi a verifier que la
/// chaine WebSocket fonctionne de bout en bout.
///
/// # Errors
/// Refus (403) pour un non-administrateur.
#[server(name = BroadcastNotification, prefix = "/sf", endpoint = "notify")]
pub async fn broadcast_notification(text: String) -> Result<BroadcastOutcome, ServerFnError> {
    use dominet_proto::{NotificationLevel, Topic, WsServerMsg};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let text = text.trim();
    if text.is_empty() || text.chars().count() > 200 {
        return Ok(BroadcastOutcome::Invalid);
    }
    let msg = WsServerMsg::Notification {
        level: NotificationLevel::Info,
        text: text.to_string(),
    };
    let receivers = state.hub.publish(&Topic::Notifications, msg);
    tracing::info!(
        admin_id = admin.id.0,
        receivers,
        chars = text.chars().count(),
        "notification broadcast"
    );
    Ok(BroadcastOutcome::Sent {
        receivers: u32::try_from(receivers).unwrap_or(u32::MAX),
    })
}

/// Injecte un etat d'appareil d'essai (administrateurs) : il est retenu comme dernier etat de `device:<id>` et diffuse aux
/// abonnes, exactement comme le fait le flux MQTT. Sert a verifier le temps reel de bout en bout et a presenter les
/// widgets d'appareil sans attendre un vrai message ; n'agit que sur un appareil existant et n'ecrit rien en base.
///
/// # Errors
/// Refus (403) pour un non-administrateur.
#[server(name = InjectDeviceState, prefix = "/sf", endpoint = "devices/inject")]
pub async fn inject_device_state(
    device_id: i64,
    state: String,
    available: bool,
) -> Result<InjectOutcome, ServerFnError> {
    use dominet_domain::device::DeviceId;
    use dominet_domain::repo::DeviceRepo;
    use dominet_proto::{DeviceStateDto, Topic, WsServerMsg};

    let app = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&app).await?;
    // Le simulateur n'agit que sur un appareil qui existe (la cle etrangere des widgets l'exige) : nom reel, rien en base.
    let Some(device) = app
        .store
        .devices()
        .get(DeviceId(device_id))
        .await
        .map_err(|e| internal("reading a device", &e))?
    else {
        return Ok(InjectOutcome::InvalidDevice);
    };
    let value = match serde_json::from_str::<serde_json::Value>(&state) {
        Ok(v) if v.is_object() && state.len() <= 4096 => v,
        _ => return Ok(InjectOutcome::InvalidState),
    };
    let msg = WsServerMsg::DeviceState(DeviceStateDto {
        device_id,
        friendly_name: device.friendly_name,
        state: value,
        available,
    });
    let receivers = app.hub.publish_retained(&Topic::Device(device_id), msg);
    tracing::info!(
        admin_id = admin.id.0,
        device_id,
        receivers,
        "test device state injected"
    );
    Ok(InjectOutcome::Sent {
        receivers: u32::try_from(receivers).unwrap_or(u32::MAX),
    })
}

/// Informations d'affichage (type, libelles, cles masquees, commandes possibles) des appareils visés par les widgets.
/// HTML assaini d'un widget `text` (vide pour les autres types).
#[cfg(feature = "ssr")]
fn text_widget_html(w: &dominet_domain::dashboard::Widget) -> String {
    use dominet_domain::dashboard::WidgetType;

    if w.widget_type != WidgetType::Text {
        return String::new();
    }
    if w.content.is_empty() {
        // Repli de l'implementation de reference : texte brut de la configuration, retours a la ligne conserves.
        let plain = w.config.get("text").and_then(|v| v.as_str()).unwrap_or("");
        dominet_server::html::plain_text_to_html(plain)
    } else {
        dominet_server::html::sanitize_widget_html(&w.content)
    }
}

/// Scenes visees par les widgets `scene` du tableau (rien a lire s'il n'y en a pas), par numero.
#[cfg(feature = "ssr")]
async fn scene_briefs(
    state: &dominet_server::AppState,
    widgets: &[dominet_domain::dashboard::Widget],
) -> Result<std::collections::HashMap<i64, dominet_proto::SceneBriefDto>, ServerFnError> {
    use dominet_domain::dashboard::WidgetType;
    use dominet_domain::repo::SceneRepo;

    if !widgets.iter().any(|w| w.widget_type == WidgetType::Scene) {
        return Ok(std::collections::HashMap::new());
    }
    Ok(state
        .store
        .scenes()
        .list()
        .await
        .map_err(|e| internal("reading the scenes", &e))?
        .into_iter()
        .map(|s| {
            (
                s.id.0,
                dominet_proto::SceneBriefDto {
                    id: s.id.0,
                    name: s.name,
                    icon: s.icon,
                    description: s.description,
                    actions: u32::try_from(s.actions.len()).unwrap_or(u32::MAX),
                    active: s.active,
                },
            )
        })
        .collect())
}

/// Cameras visees par les widgets `camera` du tableau (rien a lire s'il n'y en a pas), par identifiant : seules les cameras actives.
#[cfg(feature = "ssr")]
async fn camera_lives(
    state: &dominet_server::AppState,
    widgets: &[dominet_domain::dashboard::Widget],
) -> Result<std::collections::HashMap<uuid::Uuid, dominet_proto::CameraLiveDto>, ServerFnError> {
    use dominet_domain::dashboard::WidgetType;

    if !widgets.iter().any(|w| w.widget_type == WidgetType::Camera) {
        return Ok(std::collections::HashMap::new());
    }
    let host = crate::camera_fns::request_host().await;
    let mut out = std::collections::HashMap::new();
    for c in state
        .cameras
        .list()
        .await
        .map_err(|e| internal("reading the cameras", &e))?
        .into_iter()
        .filter(|c| c.config.active)
    {
        let dto = crate::camera_fns::live_dto(state, &c, host.as_deref()).await;
        out.insert(c.uuid.0, dto);
    }
    Ok(out)
}

#[cfg(feature = "ssr")]
async fn device_metas(
    state: &dominet_server::AppState,
    widgets: &[dominet_domain::dashboard::Widget],
) -> Result<std::collections::HashMap<i64, dominet_proto::DeviceMetaDto>, ServerFnError> {
    use dominet_domain::dashboard::WidgetType;
    use dominet_domain::repo::{DeviceLabelRepo, DeviceRepo};

    let labels: Vec<dominet_proto::LabelDto> = state
        .store
        .device_labels()
        .list()
        .await
        .map_err(|e| internal("reading the labels", &e))?
        .into_iter()
        .map(|l| dominet_proto::LabelDto {
            key: l.key,
            name: l.name,
            unit: l.unit,
        })
        .collect();
    let mut metas = std::collections::HashMap::new();
    for id in widgets
        .iter()
        .filter(|w| w.widget_type == WidgetType::Device)
        .filter_map(|w| w.device_id)
    {
        if let Some(d) = state
            .store
            .devices()
            .get(dominet_domain::device::DeviceId(id))
            .await
            .map_err(|e| internal("reading a device", &e))?
        {
            metas.insert(
                id,
                dominet_proto::DeviceMetaDto {
                    device_type: d.device_type.as_str().to_string(),
                    labels: labels.clone(),
                    hidden_keys: d.hidden_keys,
                    commandable: state.mqtt.is_some(),
                },
            );
        }
    }
    Ok(metas)
}

/// Les tableaux du compte connecte et celui qu'on ouvre : `requested` s'il lui appartient, sinon son tableau par defaut, sinon
/// le premier par nom. Un tableau d'un autre compte n'est **jamais** renvoye (l'identifiant demande est ignore).
/// Les widgets sont prepares pour l'affichage : HTML assaini, adresse d'iframe verifiee, dernier etat des appareils.
///
/// # Errors
/// Erreur interne si la base echoue. Un visiteur anonyme recoit une reponse vide.
#[server(name = DashboardPage, prefix = "/sf", endpoint = "dashboard")]
pub async fn dashboard_page(requested: Option<i64>) -> Result<DashboardPageDto, ServerFnError> {
    use dominet_domain::dashboard::{Widget, WidgetType};
    use dominet_domain::repo::{DashboardRepo, WidgetRepo};
    use dominet_domain::widget_view::{GrafanaSrc, grafana_src};
    use dominet_proto::{Topic, WidgetDto, WsServerMsg};
    use dominet_server::AppState;
    use dominet_server::auth::extract::CurrentUser;

    let state = expect_context::<AppState>();
    let Ok(CurrentUser(user)) =
        leptos_axum::extract_with_state::<CurrentUser, AppState>(&state).await
    else {
        return Ok(DashboardPageDto::default());
    };
    let boards = state
        .store
        .dashboards()
        .list_for_owner(user.id)
        .await
        .map_err(|e| internal("reading the dashboards", &e))?;
    let summaries: Vec<DashboardSummaryDto> = boards
        .iter()
        .map(|d| DashboardSummaryDto {
            id: d.id.0,
            name: d.name.clone(),
            is_default: d.is_default,
        })
        .collect();
    let chosen = requested
        .and_then(|id| summaries.iter().find(|d| d.id == id))
        .or_else(|| summaries.iter().find(|d| d.is_default))
        .or_else(|| summaries.first())
        .cloned();
    let Some(current) = chosen else {
        return Ok(DashboardPageDto {
            dashboards: summaries,
            current: None,
            widgets: vec![],
        });
    };
    let widgets = state
        .store
        .widgets()
        .list(dominet_domain::dashboard::DashboardId(current.id))
        .await
        .map_err(|e| internal("reading the widgets", &e))?;
    let grafana_base = state.settings.get_string("GRAFANA_PUBLIC_URL").await.ok();
    let metas = device_metas(&state, &widgets).await?;
    let scenes = scene_briefs(&state, &widgets).await?;
    let cameras = camera_lives(&state, &widgets).await?;
    let to_dto = |w: &Widget| {
        let (iframe_src, iframe_problem) = if w.widget_type == WidgetType::Grafana {
            match grafana_src(&w.config, w.device_id, grafana_base.as_deref()) {
                GrafanaSrc::Url(u) => (Some(u), ""),
                GrafanaSrc::NotConfigured => (None, "not_configured"),
                GrafanaSrc::Forbidden => (None, "forbidden"),
            }
        } else {
            (None, "")
        };
        let content_html = text_widget_html(w);
        let device = w
            .device_id
            .filter(|_| w.widget_type == WidgetType::Device)
            .and_then(|id| state.hub.retained(&Topic::Device(id)))
            .and_then(|m| match &*m {
                WsServerMsg::DeviceState(d) => Some(d.clone()),
                _ => None,
            });
        WidgetDto {
            id: w.id.0,
            widget_type: w.widget_type.as_str().to_string(),
            title: w.title.clone(),
            presentation: w.presentation.as_str().to_string(),
            device_id: w.device_id,
            col: w.rect.col,
            row: w.rect.row,
            width: w.rect.width,
            height: w.rect.height,
            content_html,
            iframe_src,
            iframe_problem: iframe_problem.to_string(),
            device,
            device_meta: w.device_id.and_then(|id| metas.get(&id).cloned()),
            scene: w
                .scene_id
                .filter(|_| w.widget_type == WidgetType::Scene)
                .and_then(|id| scenes.get(&id).cloned()),
            camera: w
                .camera_uuid
                .filter(|_| w.widget_type == WidgetType::Camera)
                .and_then(|u| cameras.get(&u).cloned()),
        }
    };
    Ok(DashboardPageDto {
        dashboards: summaries,
        current: Some(current),
        widgets: widgets.iter().map(to_dto).collect(),
    })
}

/// Cree un tableau pour le compte connecte (le premier devient son tableau par defaut).
///
/// # Errors
/// Refus (401) sans session ; erreur interne si la base echoue.
#[server(name = CreateDashboard, prefix = "/sf", endpoint = "dashboard/create")]
pub async fn create_dashboard(
    name: String,
) -> Result<SaveOutcome<DashboardSummaryDto>, ServerFnError> {
    use dominet_domain::dashboard::DashboardDraft;
    use dominet_domain::repo::DashboardRepo;
    use dominet_server::AppState;
    use dominet_server::auth::extract::CurrentUser;

    let state = expect_context::<AppState>();
    let Ok(CurrentUser(user)) =
        leptos_axum::extract_with_state::<CurrentUser, AppState>(&state).await
    else {
        expect_context::<leptos_axum::ResponseOptions>()
            .set_status(axum::http::StatusCode::UNAUTHORIZED);
        return Err(ServerFnError::new("authentication required"));
    };
    let draft = DashboardDraft {
        name: name.trim().to_string(),
        is_default: false,
    };
    let problems = draft.problems();
    if !problems.is_empty() {
        return Ok(SaveOutcome::Invalid(field_problems(problems)));
    }
    match state.store.dashboards().create(user.id, &draft).await {
        Ok(d) => {
            tracing::info!(user_id = user.id.0, dashboard_id = d.id.0, "tableau cree");
            Ok(SaveOutcome::Saved(DashboardSummaryDto {
                id: d.id.0,
                name: d.name,
                is_default: d.is_default,
            }))
        }
        Err(dominet_domain::RepoError::NotFound) => Ok(SaveOutcome::NotFound),
        Err(e) => Err(internal("creation d'un tableau", &e)),
    }
}
