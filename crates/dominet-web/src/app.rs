// SPDX-License-Identifier: AGPL-3.0-or-later
//! Coque HTML, fournisseur de traductions, routeur et pages.

use leptos::prelude::*;
use leptos_fluent::{leptos_fluent, move_tr};
use leptos_meta::{MetaTags, Stylesheet, Title, provide_meta_context};
use leptos_router::components::{A, Route, Router, Routes};
use leptos_router::{SsrMode, path};

use crate::account::AccountPage;
use crate::alert_rules_admin::AlertRulesAdminPage;
use crate::alerts::AlertsPage;
use crate::android::{AndroidCallsPage, AndroidDashboardPage, AndroidDevicesPage, AndroidSmsPage};
use crate::assistant::AssistantPage;
use crate::automations_admin::AutomationsAdminPage;
use crate::cameras_admin::CamerasAdminPage;
use crate::dashboard::DashboardPage;
use crate::dashboards_admin::DashboardsAdminPage;
use crate::devices_admin::{DeviceGroupsPage, DeviceLabelsPage, DevicesAdminPage};
use crate::devices_pages::{DevicePage, DevicesPage};
use crate::energy::EnergyPage;
use crate::events_page::EventsPage;
use crate::layout::{Layout, LayoutRefresh};
use crate::live::LivePage;
use crate::locations::LocationsPage;
use crate::login::LoginPage;
use crate::logs_admin::LogsPage;
use crate::mcp_admin::McpAdminPage;
use crate::messaging::{
    MessagingComposePage, MessagingDetailPage, MessagingInboxPage, MessagingSentPage,
};
use crate::mqtt_sources::MqttSourcesPage;
use crate::navbar::NavbarPage;
use crate::onvif_pages::DiscoveryPage;
use crate::recordings_page::RecordingsPage;
use crate::rules_admin::{GrafanaRulesPage, PrometheusRulesPage};
use crate::scenes::ScenesPage;
use crate::scenes_admin::ScenesAdminPage;
use crate::settings::SettingsPage;
use crate::snapshots_page::SnapshotsPage;
use crate::surveillance::SurveillancePage;
use crate::users::UsersPage;
use crate::weather_admin::WeatherStationsPage;
use crate::widgets_admin::WidgetsAdminPage;

/// Page HTML complete (rendu serveur) : scripts d'hydratation et feuille de style.
pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html>
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                <HydrationScripts options=options.clone()/>
                <MetaTags/>
            </head>
            <body>
                <App/>
            </body>
        </html>
    }
}

/// Traductions Fluent : anglais par defaut, francais complet. Langue lue dans le cookie `lang`, puis
/// `Accept-Language` cote serveur ; `<html lang>` est synchronise par `leptos-fluent`.
#[component]
fn I18nProvider(children: Children) -> impl IntoView {
    leptos_fluent! {
        children: children(),
        locales: "./locales",
        default_language: "en",
        sync_html_tag_lang: true,
        set_language_to_cookie: true,
        set_language_to_local_storage: true,
        initial_language_from_cookie: true,
        initial_language_from_local_storage: true,
        initial_language_from_navigator: true,
        initial_language_from_accept_language_header: true,
        cookie_name: "lang",
        // Fluent entoure les variables de marques bidi invisibles : on les desactive (ADR 0007).
        customise: |b| b.set_use_isolating(false),
        cookie_attrs: "Path=/; Max-Age=31536000; SameSite=Lax",
    }
}

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();
    // En-tete CSP de la reponse (avec le nonce de ce rendu) ; sans effet dans le navigateur.
    #[cfg(feature = "ssr")]
    crate::csp::apply();
    provide_context(LayoutRefresh(RwSignal::new(0)));
    view! {
        <Stylesheet id="leptos" href="/pkg/dominet.css"/>
        // Personnalisation locale (couleurs, police...) : apres la feuille principale pour la surcharger ; vide si aucun fichier.
        <Stylesheet id="custom" href="/custom.css"/>
        <I18nProvider>
            <Title text="DomiNet"/>
            <Router>
                <Layout>
                    <Routes fallback=NotFound>
                        // `Async` : tout est resolu cote serveur avant l'envoi. La barre laterale est donc dans le HTML,
                        // a sa place (en streaming hors ordre elle arrivait dans un `<template>` et le contenu glissait), et
                        // une page protegee repond par une vraie redirection 302 au lieu d'etre envoyee.
                        <Route path=path!("/") view=DashboardPage ssr=SsrMode::Async/>
                        <Route path=path!("/automation") view=DevicesPage ssr=SsrMode::Async/>
                        <Route path=path!("/automation/") view=DevicesPage ssr=SsrMode::Async/>
                        <Route path=path!("/automation/devices/:id") view=DevicePage ssr=SsrMode::Async/>
                        <Route path=path!("/automation/energy") view=EnergyPage ssr=SsrMode::Async/>
                        <Route path=path!("/automation/energy/") view=EnergyPage ssr=SsrMode::Async/>
                        <Route path=path!("/automation/scenes") view=ScenesPage ssr=SsrMode::Async/>
                        <Route path=path!("/alerts") view=AlertsPage ssr=SsrMode::Async/>
                        <Route path=path!("/android") view=AndroidDashboardPage ssr=SsrMode::Async/>
                        <Route path=path!("/android/") view=AndroidDashboardPage ssr=SsrMode::Async/>
                        <Route path=path!("/android/devices") view=AndroidDevicesPage ssr=SsrMode::Async/>
                        <Route path=path!("/android/sms") view=AndroidSmsPage ssr=SsrMode::Async/>
                        <Route path=path!("/android/calls") view=AndroidCallsPage ssr=SsrMode::Async/>
                        <Route path=path!("/messaging") view=MessagingSentPage ssr=SsrMode::Async/>
                        <Route path=path!("/messaging/") view=MessagingSentPage ssr=SsrMode::Async/>
                        <Route path=path!("/messaging/inbox") view=MessagingInboxPage ssr=SsrMode::Async/>
                        <Route path=path!("/messaging/compose") view=MessagingComposePage ssr=SsrMode::Async/>
                        <Route path=path!("/messaging/:id") view=MessagingDetailPage ssr=SsrMode::Async/>
                        <Route path=path!("/surveillance") view=SurveillancePage ssr=SsrMode::Async/>
                        <Route path=path!("/surveillance/") view=SurveillancePage ssr=SsrMode::Async/>
                        <Route path=path!("/surveillance/live/:uuid") view=LivePage ssr=SsrMode::Async/>
                        <Route path=path!("/surveillance/snapshots") view=SnapshotsPage ssr=SsrMode::Async/>
                        <Route path=path!("/surveillance/recordings/:uuid") view=RecordingsPage ssr=SsrMode::Async/>
                        <Route path=path!("/surveillance/events") view=EventsPage ssr=SsrMode::Async/>
                        <Route path=path!("/surveillance/discovery") view=DiscoveryPage ssr=SsrMode::Async/>
                        <Route path=path!("/login") view=LoginPage ssr=SsrMode::Async/>
                        <Route path=path!("/account") view=AccountPage ssr=SsrMode::Async/>
                        <Route path=path!("/admin/dashboards") view=DashboardsAdminPage ssr=SsrMode::Async/>
                        <Route path=path!("/admin/widgets") view=WidgetsAdminPage ssr=SsrMode::Async/>
                        <Route path=path!("/admin/locations") view=LocationsPage ssr=SsrMode::Async/>
                        <Route path=path!("/admin/logs") view=LogsPage ssr=SsrMode::Async/>
                        <Route path=path!("/admin/mcp") view=McpAdminPage ssr=SsrMode::Async/>
                        <Route path=path!("/admin/assistant") view=AssistantPage ssr=SsrMode::Async/>
                        <Route path=path!("/admin/cameras") view=CamerasAdminPage ssr=SsrMode::Async/>
                        <Route path=path!("/admin/mqtt-sources") view=MqttSourcesPage ssr=SsrMode::Async/>
                        <Route path=path!("/admin/devices") view=DevicesAdminPage ssr=SsrMode::Async/>
                        <Route path=path!("/admin/device-groups") view=DeviceGroupsPage ssr=SsrMode::Async/>
                        <Route path=path!("/admin/device-labels") view=DeviceLabelsPage ssr=SsrMode::Async/>
                        <Route path=path!("/admin/weather-stations") view=WeatherStationsPage ssr=SsrMode::Async/>
                        <Route path=path!("/admin/scenes") view=ScenesAdminPage ssr=SsrMode::Async/>
                        <Route path=path!("/admin/automations") view=AutomationsAdminPage ssr=SsrMode::Async/>
                        <Route path=path!("/admin/alert-rules") view=AlertRulesAdminPage ssr=SsrMode::Async/>
                        <Route path=path!("/admin/prometheus-rules") view=PrometheusRulesPage ssr=SsrMode::Async/>
                        <Route path=path!("/admin/grafana-rules") view=GrafanaRulesPage ssr=SsrMode::Async/>
                        <Route path=path!("/admin/navbar") view=NavbarPage ssr=SsrMode::Async/>
                        <Route path=path!("/admin/settings") view=SettingsPage ssr=SsrMode::Async/>
                        <Route path=path!("/admin/users") view=UsersPage ssr=SsrMode::Async/>
                    </Routes>
                </Layout>
            </Router>
        </I18nProvider>
    }
}

#[component]
fn NotFound() -> impl IntoView {
    // Le statut HTTP 404 n'existe que cote serveur.
    #[cfg(feature = "ssr")]
    {
        let response = expect_context::<leptos_axum::ResponseOptions>();
        response.set_status(axum::http::StatusCode::NOT_FOUND);
    }
    view! {
        <main class="vc-auth-wrap">
            <div class="vc-auth-card">
                <h1>{move_tr!("not-found-title")}</h1>
                <A href="/">{move_tr!("not-found-back")}</A>
            </div>
        </main>
    }
}
