// SPDX-License-Identifier: AGPL-3.0-or-later
//! Administration des comptes : `/admin/users`.
//!
//! Regles (appliquees cote serveur, l'interface ne fait que les afficher) : il doit toujours rester un administrateur
//! actif ; un administrateur ne peut pas se supprimer, se desactiver, se retrograder ni changer ici son propre mot de
//! passe (`Mon compte` sert a cela) ; reinitialiser le mot de passe d'un compte ferme toutes ses sessions.

use dominet_proto::{RoleDto, UserAdminDto, UserInput};
use leptos::prelude::*;
use leptos_fluent::move_tr;

use crate::account::format_instant;
use crate::crud::{CrudResult, CrudTable, FieldKind, FieldSpec, Labels, Row, Values};
use crate::guard::RequireAdmin;
use crate::server_fns::{delete_user, list_users, save_user};

const ROLES: &[(&str, &str)] = &[("admin", tk!("role-admin")), ("user", tk!("role-user"))];

fn row(u: &UserAdminDto) -> Row {
    let display = [u.first_name.as_str(), u.last_name.as_str()]
        .iter()
        .filter(|p| !p.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    Row {
        id: u.id,
        values: Values::from([
            ("username", u.username.clone()),
            ("display_name", display),
            ("first_name", u.first_name.clone()),
            ("last_name", u.last_name.clone()),
            ("email", u.email.clone()),
            ("mobile", u.mobile.clone()),
            (
                "role",
                if u.role == RoleDto::Admin {
                    "admin"
                } else {
                    "user"
                }
                .to_string(),
            ),
            ("is_active", u.is_active.to_string()),
            (
                "last_login",
                u.last_login_at
                    .as_deref()
                    .map_or_else(|| "-".to_string(), format_instant),
            ),
        ]),
    }
}

/// Champ de texte de base ; les cas particuliers modifient la structure obtenue.
fn field(name: &'static str, label: &'static str, kind: FieldKind) -> FieldSpec {
    FieldSpec {
        name,
        label,
        kind,
        default: "",
        required: false,
        max_len: None,
        hint: None,
        stored_flag: None,
        create_only: false,
        in_table: false,
    }
}

fn fields() -> Vec<FieldSpec> {
    vec![
        FieldSpec {
            required: true,
            max_len: Some(150),
            hint: Some(tk!("users-username-hint")),
            stored_flag: None,
            create_only: true,
            in_table: true,
            ..field("username", tk!("login-username"), FieldKind::Text)
        },
        FieldSpec {
            max_len: Some(150),
            ..field("first_name", tk!("users-first-name"), FieldKind::Text)
        },
        FieldSpec {
            max_len: Some(150),
            ..field("last_name", tk!("users-last-name"), FieldKind::Text)
        },
        FieldSpec {
            in_table: true,
            ..field("display_name", tk!("users-name"), FieldKind::Display)
        },
        FieldSpec {
            max_len: Some(254),
            in_table: true,
            ..field("email", tk!("account-email"), FieldKind::Text)
        },
        FieldSpec {
            max_len: Some(20),
            hint: Some(tk!("users-mobile-hint")),
            stored_flag: None,
            ..field("mobile", tk!("users-mobile"), FieldKind::Text)
        },
        FieldSpec {
            required: true,
            default: "user",
            in_table: true,
            ..field("role", tk!("account-role"), FieldKind::Select(ROLES))
        },
        FieldSpec {
            default: "true",
            in_table: true,
            ..field("is_active", tk!("users-active"), FieldKind::Checkbox)
        },
        FieldSpec {
            hint: Some(tk!("users-password-hint")),
            stored_flag: None,
            ..field("password", tk!("login-password"), FieldKind::Password)
        },
        FieldSpec {
            in_table: true,
            ..field("last_login", tk!("account-last-login"), FieldKind::Display)
        },
    ]
}

#[component]
pub fn UsersPage() -> impl IntoView {
    view! {
        <RequireAdmin>
            <main class="vc-container">
                <UsersAdmin/>
            </main>
        </RequireAdmin>
    }
}

#[component]
fn UsersAdmin() -> impl IntoView {
    let version = RwSignal::new(0u32);
    let list = Resource::new(
        move || version.get(),
        |_| async { list_users().await.map_err(|_| ()) },
    );
    let rows = Signal::derive(move || match list.get() {
        Some(Ok(items)) => items.iter().map(row).collect(),
        _ => vec![],
    });

    let save = Action::new(|(id, v): &(Option<i64>, Values)| {
        let get = |k: &str| v.get(k).cloned().unwrap_or_default();
        let input = UserInput {
            id: *id,
            username: get("username"),
            first_name: get("first_name"),
            last_name: get("last_name"),
            email: get("email"),
            mobile: get("mobile"),
            role: get("role"),
            is_active: get("is_active") == "true",
            password: get("password"),
        };
        async move {
            save_user(input)
                .await
                .map_or(CrudResult::Failed, CrudResult::from)
        }
    });
    let remove = Action::new(|id: &i64| {
        let id = *id;
        async move {
            delete_user(id)
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
                    title: tk!("users-title"),
                    add: tk!("users-add"),
                    edit: tk!("users-edit"),
                    empty: tk!("users-empty"),
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
