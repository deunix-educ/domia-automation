// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tests des depots PostgreSQL via les ports du domaine (base isolee par test, `make dev-up`).

use dominet_domain::RepoError;
use dominet_domain::dashboard::{
    DashboardDraft, DashboardId, GridRect, Presentation, WidgetDraft, WidgetId, WidgetType,
};
use dominet_domain::location::{LocationDraft, LocationId};
use dominet_domain::navbar::{NavPosition, NavbarDraft, NavbarItemId};
use dominet_domain::repo::{
    DashboardRepo, DeviceRepo, LocationRepo, NavbarRepo, SettingRepo, UserRepo, WidgetRepo,
};
use dominet_domain::setting::{SettingDraft, SettingKey, SettingType};
use dominet_domain::user::{NewUser, Role, UserId, UserProfile, Username};
use dominet_store::PgStore;
use sqlx::PgPool;

fn new_user(name: &str, role: Role) -> NewUser {
    NewUser {
        username: Username::parse(name).unwrap(),
        email: format!("{name}@example.org"),
        first_name: String::new(),
        last_name: String::new(),
        mobile: String::new(),
        role,
        is_active: true,
        password_hash: format!("hash-of-{name}"),
    }
}

fn profile(role: Role, active: bool) -> UserProfile {
    UserProfile {
        email: "new@example.org".into(),
        first_name: "Alice".into(),
        last_name: "Martin".into(),
        mobile: "+33612345678".into(),
        role,
        is_active: active,
    }
}

// ---------------------------------------------------------------- utilisateurs

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn user_create_and_lookup(pool: PgPool) {
    let store = PgStore::new(pool);
    let created = store
        .users()
        .create(&new_user("Bob", Role::Admin))
        .await
        .unwrap();
    assert_eq!(
        (created.username.as_str(), created.role, created.is_active),
        ("Bob", Role::Admin, true)
    );
    assert!(created.last_login_at.is_none());

    assert_eq!(
        store.users().find_by_id(created.id).await.unwrap().unwrap(),
        created
    );
    for spelling in ["Bob", "bob", "BOB"] {
        assert_eq!(
            store
                .users()
                .find_by_username(spelling)
                .await
                .unwrap()
                .unwrap()
                .id,
            created.id,
            "{spelling}"
        );
    }
    assert!(
        store
            .users()
            .find_by_username("alice")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .users()
            .find_by_id(UserId(999_999))
            .await
            .unwrap()
            .is_none()
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn user_duplicate_username_is_a_conflict_whatever_the_case(pool: PgPool) {
    let store = PgStore::new(pool);
    store
        .users()
        .create(&new_user("Bob", Role::User))
        .await
        .unwrap();
    let err = store
        .users()
        .create(&new_user("bOB", Role::User))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, RepoError::Conflict { constraint } if constraint == "users_username_lower_key"),
        "{err:?}"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn credentials_carry_the_hash_and_are_case_insensitive(pool: PgPool) {
    let store = PgStore::new(pool);
    store
        .users()
        .create(&new_user("Bob", Role::User))
        .await
        .unwrap();
    let c = store
        .users()
        .find_credentials("BOB")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(c.password_hash, "hash-of-Bob");
    assert_eq!(c.user.username, "Bob");
    assert!(
        store
            .users()
            .find_credentials("nobody")
            .await
            .unwrap()
            .is_none()
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn users_are_listed_alphabetically_ignoring_case(pool: PgPool) {
    let store = PgStore::new(pool);
    for n in ["charlie", "Alice", "bob"] {
        store
            .users()
            .create(&new_user(n, Role::User))
            .await
            .unwrap();
    }
    let names: Vec<String> = store
        .users()
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|u| u.username)
        .collect();
    assert_eq!(names, ["Alice", "bob", "charlie"]);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn user_profile_update(pool: PgPool) {
    let store = PgStore::new(pool);
    let u = store
        .users()
        .create(&new_user("bob", Role::User))
        .await
        .unwrap();
    let updated = store
        .users()
        .update_profile(u.id, &profile(Role::Admin, false))
        .await
        .unwrap();
    assert_eq!(updated.username, "bob", "le nom ne change pas");
    assert_eq!(
        (updated.role, updated.is_active, updated.mobile.as_str()),
        (Role::Admin, false, "+33612345678")
    );
    assert_eq!(updated.display_name(), "Alice Martin");
    assert!(updated.updated_at >= u.updated_at);
    assert!(matches!(
        store
            .users()
            .update_profile(UserId(424_242), &profile(Role::User, true))
            .await,
        Err(RepoError::NotFound)
    ));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn password_change_replaces_hash_and_dates_it(pool: PgPool) {
    let store = PgStore::new(pool);
    let u = store
        .users()
        .create(&new_user("bob", Role::User))
        .await
        .unwrap();
    store
        .users()
        .set_password_hash(u.id, "new-hash")
        .await
        .unwrap();
    let c = store
        .users()
        .find_credentials("bob")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(c.password_hash, "new-hash");
    assert!(c.user.password_changed_at >= u.password_changed_at);
    assert!(matches!(
        store.users().set_password_hash(UserId(424_242), "x").await,
        Err(RepoError::NotFound)
    ));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn last_login_is_recorded(pool: PgPool) {
    let store = PgStore::new(pool);
    let u = store
        .users()
        .create(&new_user("bob", Role::User))
        .await
        .unwrap();
    store.users().touch_last_login(u.id).await.unwrap();
    assert!(
        store
            .users()
            .find_by_id(u.id)
            .await
            .unwrap()
            .unwrap()
            .last_login_at
            .is_some()
    );
    assert!(matches!(
        store.users().touch_last_login(UserId(424_242)).await,
        Err(RepoError::NotFound)
    ));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn user_delete_then_not_found(pool: PgPool) {
    let store = PgStore::new(pool);
    let u = store
        .users()
        .create(&new_user("bob", Role::User))
        .await
        .unwrap();
    store.users().delete(u.id).await.unwrap();
    assert!(store.users().find_by_id(u.id).await.unwrap().is_none());
    assert!(matches!(
        store.users().delete(u.id).await,
        Err(RepoError::NotFound)
    ));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn only_active_admins_are_counted(pool: PgPool) {
    let store = PgStore::new(pool);
    assert_eq!(store.users().count_active_admins().await.unwrap(), 0);
    let a = store
        .users()
        .create(&new_user("root1", Role::Admin))
        .await
        .unwrap();
    let b = store
        .users()
        .create(&new_user("root2", Role::Admin))
        .await
        .unwrap();
    store
        .users()
        .create(&new_user("simple", Role::User))
        .await
        .unwrap();
    assert_eq!(store.users().count_active_admins().await.unwrap(), 2);
    store
        .users()
        .update_profile(b.id, &profile(Role::Admin, false))
        .await
        .unwrap();
    assert_eq!(
        store.users().count_active_admins().await.unwrap(),
        1,
        "un admin desactive ne compte pas"
    );
    // Le dernier administrateur actif ne peut plus etre retrograde (voir les tests « last_admin »).
    assert!(matches!(
        store
            .users()
            .update_profile(a.id, &profile(Role::User, true))
            .await,
        Err(RepoError::LastAdmin)
    ));
    assert_eq!(store.users().count_active_admins().await.unwrap(), 1);
}

async fn admins(store: &PgStore, names: &[&str]) -> Vec<UserId> {
    let mut ids = vec![];
    for n in names {
        ids.push(
            store
                .users()
                .create(&new_user(n, Role::Admin))
                .await
                .unwrap()
                .id,
        );
    }
    ids
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_last_active_admin_cannot_be_demoted_deactivated_or_deleted(pool: PgPool) {
    let store = PgStore::new(pool);
    let ids = admins(&store, &["root1", "root2"]).await;
    let (a, b) = (ids[0], ids[1]);
    let simple = store
        .users()
        .create(&new_user("simple", Role::User))
        .await
        .unwrap();

    // Tant qu'il en reste un autre, tout est permis.
    store
        .users()
        .update_profile(b, &profile(Role::Admin, false))
        .await
        .unwrap();
    // `a` est maintenant le dernier : demotion, desactivation et suppression sont refusees...
    for p in [
        profile(Role::User, true),
        profile(Role::Admin, false),
        profile(Role::User, false),
    ] {
        assert!(
            matches!(
                store.users().update_profile(a, &p).await,
                Err(RepoError::LastAdmin)
            ),
            "{p:?}"
        );
    }
    assert!(matches!(
        store.users().delete(a).await,
        Err(RepoError::LastAdmin)
    ));
    // ... sans rien modifier,
    let a_now = store.users().find_by_id(a).await.unwrap().unwrap();
    assert!(a_now.role.is_admin() && a_now.is_active);
    // ... mais une modification qui le laisse administrateur actif passe, et supprimer les autres aussi.
    store
        .users()
        .update_profile(a, &profile(Role::Admin, true))
        .await
        .unwrap();
    store.users().delete(simple.id).await.unwrap();
    store.users().delete(b).await.unwrap(); // `b` etait deja desactive : ne compte pas
    // `a` reste le dernier administrateur actif.
    assert!(matches!(
        store.users().delete(a).await,
        Err(RepoError::LastAdmin)
    ));
    // Un compte inexistant reste « introuvable », pas « dernier administrateur ».
    assert!(matches!(
        store.users().delete(UserId(424_242)).await,
        Err(RepoError::NotFound)
    ));
    assert!(matches!(
        store
            .users()
            .update_profile(UserId(424_242), &profile(Role::User, true))
            .await,
        Err(RepoError::NotFound)
    ));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn two_admins_demoting_each_other_at_once_never_leave_zero(pool: PgPool) {
    let store = PgStore::new(pool);
    let ids = admins(&store, &["root1", "root2"]).await;
    // Sans verrou, chacun verrait « il reste l'autre » et les deux passeraient. On rejoue la course plusieurs fois.
    for _ in 0..25 {
        let (s1, s2) = (store.clone(), store.clone());
        let (a, b) = (ids[0], ids[1]);
        let (r1, r2) = tokio::join!(
            async move {
                s1.users()
                    .update_profile(a, &profile(Role::User, true))
                    .await
            },
            async move {
                s2.users()
                    .update_profile(b, &profile(Role::User, true))
                    .await
            },
        );
        assert!(r1.is_ok() || r2.is_ok(), "au moins une demotion passe");
        assert!(
            r1.is_err() || r2.is_err(),
            "les deux ne peuvent pas passer : {r1:?} / {r2:?}"
        );
        assert_eq!(store.users().count_active_admins().await.unwrap(), 1);
        // Remet les deux en administrateurs pour le tour suivant.
        for id in ids.clone() {
            store
                .users()
                .update_profile(id, &profile(Role::Admin, true))
                .await
                .unwrap();
        }
    }
}

// ---------------------------------------------------------------- reglages

fn draft(key: &str, value: &str, ty: SettingType, category: &str, secret: bool) -> SettingDraft {
    SettingDraft {
        key: SettingKey::parse(key).unwrap(),
        value: value.into(),
        value_type: ty,
        category: category.into(),
        description: format!("desc {key}"),
        is_secret: secret,
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn setting_upsert_creates_then_replaces(pool: PgPool) {
    let store = PgStore::new(pool);
    let d = draft(
        "MOTION_THRESHOLD",
        "0.01",
        SettingType::Float,
        "Surveillance",
        false,
    );
    let first = store.settings().upsert(&d, &d.value).await.unwrap();
    assert_eq!(
        (first.value.as_str(), first.value_type),
        ("0.01", SettingType::Float)
    );

    let d2 = draft(
        "MOTION_THRESHOLD",
        "0.05",
        SettingType::Float,
        "Alerts",
        false,
    );
    let second = store.settings().upsert(&d2, &d2.value).await.unwrap();
    assert_eq!(
        (second.value.as_str(), second.category.as_str()),
        ("0.05", "Alerts")
    );
    assert!(second.updated_at >= first.updated_at);
    assert_eq!(
        store.settings().list().await.unwrap().len(),
        1,
        "pas de doublon"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn setting_insert_if_absent_never_overwrites(pool: PgPool) {
    let store = PgStore::new(pool);
    let d = draft(
        "Z2M_BASE_TOPIC",
        "zigbee2mqtt",
        SettingType::Str,
        "MQTT",
        false,
    );
    assert!(
        store
            .settings()
            .insert_if_absent(&d, &d.value)
            .await
            .unwrap()
    );
    let other = draft("Z2M_BASE_TOPIC", "autre", SettingType::Str, "MQTT", false);
    assert!(
        !store
            .settings()
            .insert_if_absent(&other, &other.value)
            .await
            .unwrap()
    );
    assert_eq!(
        store
            .settings()
            .get("Z2M_BASE_TOPIC")
            .await
            .unwrap()
            .unwrap()
            .value,
        "zigbee2mqtt"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn settings_are_listed_by_category_then_key(pool: PgPool) {
    let store = PgStore::new(pool);
    for (k, c) in [
        ("B_KEY", "Alerts"),
        ("A_KEY", "Surveillance"),
        ("A_OTHER", "Alerts"),
    ] {
        let d = draft(k, "x", SettingType::Str, c, false);
        store.settings().upsert(&d, "x").await.unwrap();
    }
    let keys: Vec<String> = store
        .settings()
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|s| s.key)
        .collect();
    assert_eq!(keys, ["A_OTHER", "B_KEY", "A_KEY"]);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn secret_settings_store_what_they_are_given_and_mask_it(pool: PgPool) {
    let store = PgStore::new(pool);
    let d = draft(
        "SMS_FREE_KEY",
        "clair",
        SettingType::Str,
        "Notifications",
        true,
    );
    // Le depot stocke la valeur deja chiffree que l'appelant lui fournit (lot 6), jamais le clair.
    let s = store
        .settings()
        .upsert(&d, "v1:chiffre-opaque")
        .await
        .unwrap();
    assert!(s.is_secret);
    assert_eq!(s.value, "v1:chiffre-opaque");
    assert_eq!(s.display_value(), "********");
    assert!(!format!("{s:?}").contains("clair"));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn setting_get_and_delete(pool: PgPool) {
    let store = PgStore::new(pool);
    assert!(store.settings().get("NOPE").await.unwrap().is_none());
    let d = draft("TMP_KEY_X", "1", SettingType::Int, "Interface", false);
    store.settings().upsert(&d, "1").await.unwrap();
    assert_eq!(
        store
            .settings()
            .get("TMP_KEY_X")
            .await
            .unwrap()
            .unwrap()
            .typed()
            .unwrap(),
        dominet_domain::setting::SettingValue::Int(1)
    );
    store.settings().delete("TMP_KEY_X").await.unwrap();
    assert!(matches!(
        store.settings().delete("TMP_KEY_X").await,
        Err(RepoError::NotFound)
    ));
}

// ---------------------------------------------------------------- navigation

fn nav(label: &str, pos: NavPosition, order: i16) -> NavbarDraft {
    NavbarDraft {
        icon_name: "video".into(),
        label: label.into(),
        url: format!("/{}/", label.to_lowercase()),
        url_target: "_self".into(),
        admin_only: pos == NavPosition::Bottom,
        position: pos,
        sort_order: order,
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn navbar_lists_top_first_then_bottom_ordered(pool: PgPool) {
    let store = PgStore::new(pool);
    for (l, p, o) in [
        ("Messages", NavPosition::Bottom, 20),
        ("Android", NavPosition::Bottom, 10),
        ("Observabilite", NavPosition::Top, 10),
        ("Surveillance", NavPosition::Top, 1),
        ("Automation", NavPosition::Top, 1),
    ] {
        store.navbar().create(&nav(l, p, o)).await.unwrap();
    }
    let labels: Vec<String> = store
        .navbar()
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|i| i.label)
        .collect();
    assert_eq!(
        labels,
        [
            "Automation",
            "Surveillance",
            "Observabilite",
            "Android",
            "Messages"
        ]
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn navbar_crud(pool: PgPool) {
    let store = PgStore::new(pool);
    let item = store
        .navbar()
        .create(&nav("Surveillance", NavPosition::Top, 1))
        .await
        .unwrap();
    assert_eq!(store.navbar().get(item.id).await.unwrap().unwrap(), item);
    let changed = store
        .navbar()
        .update(
            item.id,
            &NavbarDraft {
                label: "Cameras".into(),
                admin_only: true,
                ..nav("x", NavPosition::Top, 3)
            },
        )
        .await
        .unwrap();
    assert_eq!(
        (
            changed.label.as_str(),
            changed.admin_only,
            changed.sort_order
        ),
        ("Cameras", true, 3)
    );
    assert!(matches!(
        store
            .navbar()
            .update(NavbarItemId(424_242), &nav("x", NavPosition::Top, 0))
            .await,
        Err(RepoError::NotFound)
    ));
    store.navbar().delete(item.id).await.unwrap();
    assert!(matches!(
        store.navbar().delete(item.id).await,
        Err(RepoError::NotFound)
    ));
    assert!(store.navbar().get(item.id).await.unwrap().is_none());
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn navbar_insert_if_absent_matches_on_position_and_label(pool: PgPool) {
    let store = PgStore::new(pool);
    assert!(
        store
            .navbar()
            .insert_if_absent(&nav("Messages", NavPosition::Bottom, 20))
            .await
            .unwrap()
    );
    assert!(
        !store
            .navbar()
            .insert_if_absent(&nav("Messages", NavPosition::Bottom, 99))
            .await
            .unwrap(),
        "meme position et libelle"
    );
    assert!(
        store
            .navbar()
            .insert_if_absent(&nav("Messages", NavPosition::Top, 1))
            .await
            .unwrap(),
        "autre position : nouvelle entree"
    );
    assert_eq!(store.navbar().list().await.unwrap().len(), 2);
}

// ---------------------------------------------------------------- emplacements

fn loc(name: &str, active: bool) -> LocationDraft {
    LocationDraft {
        name: name.into(),
        description: format!("la piece {name}"),
        active,
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn location_crud_and_order(pool: PgPool) {
    let store = PgStore::new(pool);
    let salon = store.locations().create(&loc("Salon", true)).await.unwrap();
    store
        .locations()
        .create(&loc("Cuisine", true))
        .await
        .unwrap();
    let names: Vec<String> = store
        .locations()
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|l| l.name)
        .collect();
    assert_eq!(names, ["Cuisine", "Salon"]);

    let moved = store
        .locations()
        .update(salon.id, &loc("Sejour", false))
        .await
        .unwrap();
    assert_eq!((moved.name.as_str(), moved.active), ("Sejour", false));
    assert_eq!(
        store.locations().get(salon.id).await.unwrap().unwrap(),
        moved
    );
    assert!(matches!(
        store
            .locations()
            .update(LocationId(424_242), &loc("x", true))
            .await,
        Err(RepoError::NotFound)
    ));
    store.locations().delete(salon.id).await.unwrap();
    assert!(matches!(
        store.locations().delete(salon.id).await,
        Err(RepoError::NotFound)
    ));
}

// ---------------------------------------------------------------- tableaux de bord et widgets

fn dash(name: &str, is_default: bool) -> DashboardDraft {
    DashboardDraft {
        name: name.into(),
        is_default,
    }
}

fn text_at(col: i16, row: i16, width: i16, height: i16) -> WidgetDraft {
    let mut d = WidgetDraft::new(WidgetType::Text);
    d.content = "<p>x</p>".into();
    d.rect = GridRect {
        col,
        row,
        width,
        height,
    };
    d
}

async fn owner(store: &PgStore, name: &str) -> UserId {
    store
        .users()
        .create(&new_user(name, Role::User))
        .await
        .unwrap()
        .id
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_first_dashboard_of_an_account_is_its_default(pool: PgPool) {
    let store = PgStore::new(pool);
    let (bob, eve) = (owner(&store, "bob").await, owner(&store, "eve").await);
    let salon = store
        .dashboards()
        .create(bob, &dash("  Salon ", false))
        .await
        .unwrap();
    assert_eq!(
        (salon.name.as_str(), salon.owner, salon.is_default),
        ("Salon", bob, true),
        "rogne, et premier = defaut"
    );
    let cuisine = store
        .dashboards()
        .create(bob, &dash("Cuisine", false))
        .await
        .unwrap();
    assert!(!cuisine.is_default);
    store
        .dashboards()
        .create(eve, &dash("Bureau", false))
        .await
        .unwrap();

    let mine = store.dashboards().list_for_owner(bob).await.unwrap();
    assert_eq!(
        mine.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
        ["Cuisine", "Salon"],
        "par nom, et seulement les siens"
    );
    assert_eq!(store.dashboards().list_all().await.unwrap().len(), 3);
    assert!(
        store
            .dashboards()
            .get(DashboardId(424_242))
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        store
            .dashboards()
            .create(UserId(424_242), &dash("X", false))
            .await,
        Err(RepoError::NotFound)
    ));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn designating_a_default_moves_the_status_and_it_cannot_be_unset_directly(pool: PgPool) {
    let store = PgStore::new(pool);
    let (bob, eve) = (owner(&store, "bob").await, owner(&store, "eve").await);
    let a = store
        .dashboards()
        .create(bob, &dash("A", false))
        .await
        .unwrap();
    let b = store
        .dashboards()
        .create(bob, &dash("B", false))
        .await
        .unwrap();
    let other = store
        .dashboards()
        .create(eve, &dash("E", false))
        .await
        .unwrap();
    assert!(a.is_default && !b.is_default);

    let b = store
        .dashboards()
        .update(b.id, &dash("B", true))
        .await
        .unwrap();
    assert!(b.is_default);
    let defaults = |list: Vec<dominet_domain::dashboard::Dashboard>| {
        list.into_iter()
            .filter(|d| d.is_default)
            .map(|d| d.name)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        defaults(store.dashboards().list_for_owner(bob).await.unwrap()),
        ["B"],
        "un seul defaut"
    );
    assert!(
        store
            .dashboards()
            .get(other.id)
            .await
            .unwrap()
            .unwrap()
            .is_default,
        "l'autre compte n'est pas touche"
    );

    // Demander `false` sur le tableau par defaut ne le retire pas ; renommer ne change pas le statut.
    let kept = store
        .dashboards()
        .update(b.id, &dash("B2", false))
        .await
        .unwrap();
    assert_eq!((kept.name.as_str(), kept.is_default), ("B2", true));
    assert!(matches!(
        store
            .dashboards()
            .update(DashboardId(424_242), &dash("X", false))
            .await,
        Err(RepoError::NotFound)
    ));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn concurrent_default_changes_leave_exactly_one_default(pool: PgPool) {
    let store = PgStore::new(pool);
    let bob = owner(&store, "bob").await;
    let mut ids = vec![];
    for n in ["A", "B", "C", "D"] {
        ids.push(
            store
                .dashboards()
                .create(bob, &dash(n, false))
                .await
                .unwrap()
                .id,
        );
    }
    for _ in 0..10 {
        let tasks: Vec<_> = ids
            .iter()
            .map(|&id| {
                let s = store.clone();
                tokio::spawn(async move { s.dashboards().update(id, &dash("x", true)).await })
            })
            .collect();
        for t in tasks {
            t.await.unwrap().unwrap();
        }
        let n = store
            .dashboards()
            .list_for_owner(bob)
            .await
            .unwrap()
            .iter()
            .filter(|d| d.is_default)
            .count();
        assert_eq!(n, 1, "un seul tableau par defaut apres une course");
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn widgets_are_stored_listed_in_grid_order_and_updated(pool: PgPool) {
    let store = PgStore::new(pool);
    let bob = owner(&store, "bob").await;
    let d = store
        .dashboards()
        .create(bob, &dash("Salon", false))
        .await
        .unwrap();
    let low = store
        .widgets()
        .create(d.id, &text_at(4, 2, 2, 2))
        .await
        .unwrap();
    let mut dev = WidgetDraft::new(WidgetType::Device);
    // Depuis R3, la reference doit viser un appareil existant.
    let sonde = store
        .devices()
        .create(&dominet_domain::device::DeviceDraft::new("Lampe"))
        .await
        .unwrap();
    dev.device_id = Some(sonde.id.0);
    dev.presentation = Presentation::DeviceLight;
    dev.title = " Lampe ".into();
    dev.rect = GridRect {
        col: 2,
        row: 0,
        width: 2,
        height: 2,
    };
    let dev = store.widgets().create(d.id, &dev).await.unwrap();
    let left = store
        .widgets()
        .create(d.id, &text_at(0, 0, 2, 2))
        .await
        .unwrap();

    assert_eq!(
        (dev.title.as_str(), dev.device_id, dev.presentation),
        ("Lampe", Some(sonde.id.0), Presentation::DeviceLight)
    );
    let order: Vec<WidgetId> = store
        .widgets()
        .list(d.id)
        .await
        .unwrap()
        .iter()
        .map(|w| w.id)
        .collect();
    assert_eq!(order, [left.id, dev.id, low.id], "par ligne puis colonne");
    assert_eq!(
        store.widgets().get(dev.id).await.unwrap().unwrap(),
        dev,
        "aller-retour fidele"
    );

    let mut graf = WidgetDraft::new(WidgetType::Grafana);
    graf.config = serde_json::json!({"panel_url": "https://g.example/d-solo/a?panelId=1"});
    graf.rect = GridRect {
        col: 8,
        row: 0,
        width: 4,
        height: 3,
    };
    let graf = store.widgets().create(d.id, &graf).await.unwrap();
    assert_eq!(
        graf.config["panel_url"],
        "https://g.example/d-solo/a?panelId=1"
    );

    let mut edited = low.to_draft();
    edited.title = "Note".into();
    edited.content = "<p>y</p>".into();
    let edited = store.widgets().update(low.id, &edited).await.unwrap();
    assert_eq!(
        (edited.title.as_str(), edited.content.as_str(), edited.rect),
        ("Note", "<p>y</p>", low.rect)
    );
    assert!(matches!(
        store
            .widgets()
            .update(WidgetId(424_242), &low.to_draft())
            .await,
        Err(RepoError::NotFound)
    ));
    assert!(
        store
            .widgets()
            .get(WidgetId(424_242))
            .await
            .unwrap()
            .is_none()
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_overlapping_widget_is_refused_whether_created_updated_or_moved(pool: PgPool) {
    let store = PgStore::new(pool);
    let bob = owner(&store, "bob").await;
    let d = store
        .dashboards()
        .create(bob, &dash("Salon", false))
        .await
        .unwrap();
    let a = store
        .widgets()
        .create(d.id, &text_at(0, 0, 2, 2))
        .await
        .unwrap();
    let b = store
        .widgets()
        .create(d.id, &text_at(2, 0, 2, 2))
        .await
        .unwrap();
    let overlap = |r: Result<(), RepoError>| matches!(r, Err(RepoError::Conflict { ref constraint }) if constraint == "widget_no_overlap");

    assert!(overlap(
        store
            .widgets()
            .create(d.id, &text_at(1, 1, 2, 2))
            .await
            .map(|_| ())
    ));
    assert!(
        overlap(store.widgets().move_to(b.id, 1, 0).await.map(|_| ())),
        "deplacement sur un voisin"
    );
    let mut grow = b.to_draft();
    grow.rect.width = 12;
    grow.rect.col = 0;
    assert!(
        overlap(store.widgets().update(b.id, &grow).await.map(|_| ())),
        "agrandissement sur un voisin"
    );
    // Rien n'a bouge.
    assert_eq!(
        store.widgets().get(b.id).await.unwrap().unwrap().rect,
        b.rect
    );
    assert_eq!(
        store.widgets().get(a.id).await.unwrap().unwrap().rect,
        a.rect
    );
    // Un deplacement libre passe, y compris vers une cellule que le widget occupait deja en partie.
    let moved = store.widgets().move_to(b.id, 3, 0).await.unwrap();
    assert_eq!(
        (moved.rect.col, moved.rect.row, moved.rect.width),
        (3, 0, 2),
        "taille inchangee"
    );
    assert!(matches!(
        store.widgets().move_to(WidgetId(424_242), 0, 0).await,
        Err(RepoError::NotFound)
    ));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn concurrent_widget_placements_on_the_same_cells_let_exactly_one_win(pool: PgPool) {
    let store = PgStore::new(pool);
    let bob = owner(&store, "bob").await;
    let d = store
        .dashboards()
        .create(bob, &dash("Salon", false))
        .await
        .unwrap();
    for round in 0..10_i16 {
        let row = round * 2;
        let tasks: Vec<_> = (0..6)
            .map(|_| {
                let s = store.clone();
                let id = d.id;
                tokio::spawn(async move { s.widgets().create(id, &text_at(0, row, 2, 2)).await })
            })
            .collect();
        let mut wins = 0;
        for t in tasks {
            match t.await.unwrap() {
                Ok(_) => wins += 1,
                Err(RepoError::Conflict { constraint }) => {
                    assert_eq!(constraint, "widget_no_overlap");
                }
                Err(e) => panic!("{e:?}"),
            }
        }
        assert_eq!(wins, 1, "tour {round} : un seul widget sur ces cellules");
    }
    assert_eq!(store.widgets().list(d.id).await.unwrap().len(), 10);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn deleting_a_dashboard_deletes_its_widgets_and_widgets_delete_alone(pool: PgPool) {
    let store = PgStore::new(pool);
    let bob = owner(&store, "bob").await;
    let d = store
        .dashboards()
        .create(bob, &dash("Salon", false))
        .await
        .unwrap();
    let w1 = store
        .widgets()
        .create(d.id, &text_at(0, 0, 2, 2))
        .await
        .unwrap();
    let w2 = store
        .widgets()
        .create(d.id, &text_at(2, 0, 2, 2))
        .await
        .unwrap();
    store.widgets().delete(w1.id).await.unwrap();
    assert!(matches!(
        store.widgets().delete(w1.id).await,
        Err(RepoError::NotFound)
    ));
    // La place liberee peut etre reutilisee.
    store
        .widgets()
        .create(d.id, &text_at(0, 0, 2, 2))
        .await
        .unwrap();
    store.dashboards().delete(d.id).await.unwrap();
    assert!(
        store.widgets().get(w2.id).await.unwrap().is_none(),
        "widgets supprimes avec le tableau"
    );
    assert_eq!(store.widgets().list(d.id).await.unwrap().len(), 0);
    assert!(matches!(
        store.dashboards().delete(d.id).await,
        Err(RepoError::NotFound)
    ));
    // Un tableau inexistant.
    assert!(matches!(
        store.widgets().create(d.id, &text_at(0, 0, 2, 2)).await,
        Err(RepoError::NotFound)
    ));
}
