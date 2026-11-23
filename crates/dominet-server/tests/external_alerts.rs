// SPDX-License-Identifier: AGPL-3.0-or-later
//! Reading of Prometheus and Grafana alerts (R13 lot E1) against fake HTTP services.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use dominet_server::external_alerts::{ExternalAlerts, Source, SourceState};
use serde_json::{Value, json};

async fn serve(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn prometheus(alerts: Value) -> Router {
    Router::new().route(
        "/api/v1/alerts",
        get(move || {
            let body = alerts.clone();
            async move { Json(json!({"status": "success", "data": {"alerts": body}})) }
        }),
    )
}

/// Grafana: refuses without the right account, otherwise answers with the list.
fn grafana(alerts: Value, up: Arc<AtomicBool>) -> Router {
    Router::new()
        .route(
            "/api/alertmanager/grafana/api/v2/alerts",
            get(
                |State((alerts, up)): State<(Value, Arc<AtomicBool>)>, headers: HeaderMap| async move {
                    let auth = headers
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or_default()
                        .to_string();
                    // admin:secret
                    if !up.load(Ordering::SeqCst) {
                        return Err(StatusCode::BAD_GATEWAY);
                    }
                    if auth != "Basic YWRtaW46c2VjcmV0" {
                        return Err(StatusCode::UNAUTHORIZED);
                    }
                    Ok(Json(alerts))
                },
            ),
        )
        .with_state((alerts, up))
}

#[tokio::test]
async fn prometheus_and_grafana_alerts_are_read_counted_and_ordered() {
    let prom = serve(prometheus(json!([
        {"state": "firing", "activeAt": "2026-11-18T08:00:00Z",
         "labels": {"alertname": "MqttDown", "severity": "critical"}, "annotations": {"summary": "MQTT hors ligne"}},
        {"state": "pending", "labels": {"alertname": "Lent"}, "annotations": {}}
    ])))
    .await;
    let graf = serve(grafana(
        json!([
            {"status": {"state": "active"}, "startsAt": "2026-11-18T09:00:00Z",
             "labels": {"alertname": "Canicule", "severity": "warning"}, "annotations": {"summary": "36 degres"}},
            {"status": {"state": "suppressed"}, "labels": {"alertname": "Muette"}, "annotations": {}}
        ]),
        Arc::new(AtomicBool::new(true)),
    ))
    .await;
    let alerts =
        ExternalAlerts::with_endpoints(Some(prom), Some((graf, "admin".into(), "secret".into())));
    assert!(alerts.is_configured());
    assert_eq!(
        alerts.firing(),
        0,
        "rien n'est connu avant la premiere lecture"
    );

    assert!(alerts.refresh().await);
    assert_eq!(alerts.firing(), 2);
    let all = alerts.snapshot().alerts();
    assert_eq!(all[0].name, "MqttDown");
    assert_eq!(all[0].source, Source::Prometheus);
    assert_eq!(all[1].name, "Canicule");
    assert_eq!(all[1].source, Source::Grafana);
    assert_eq!(all[1].since.as_deref(), Some("2026-11-18T09:00:00Z"));
    // A second identical read changes nothing: the badge is not rebroadcast for nothing.
    assert!(!alerts.refresh().await);
}

#[tokio::test]
async fn an_unreachable_or_refusing_source_is_reported_not_hidden() {
    let up = Arc::new(AtomicBool::new(true));
    let graf = serve(grafana(json!([]), up.clone())).await;
    // Wrong password: Grafana refuses, the source is unreachable (and not "no alert").
    let wrong =
        ExternalAlerts::with_endpoints(None, Some((graf.clone(), "admin".into(), "faux".into())));
    wrong.refresh().await;
    assert_eq!(wrong.snapshot().grafana, SourceState::Unreachable);
    assert_eq!(wrong.snapshot().prometheus, SourceState::NotConfigured);

    // Right account: an empty list means "all is well". Then the service goes down: we say so.
    let right = ExternalAlerts::with_endpoints(None, Some((graf, "admin".into(), "secret".into())));
    right.refresh().await;
    assert_eq!(right.snapshot().grafana, SourceState::Ok(vec![]));
    up.store(false, Ordering::SeqCst);
    assert!(right.refresh().await);
    assert_eq!(right.snapshot().grafana, SourceState::Unreachable);
    assert_eq!(right.firing(), 0);

    // Nothing configured: nothing is read.
    let none = ExternalAlerts::with_endpoints(None, None);
    assert!(!none.is_configured());
    assert!(!none.refresh().await);
}
