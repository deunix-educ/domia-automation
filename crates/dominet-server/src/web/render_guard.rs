// SPDX-License-Identifier: AGPL-3.0-or-later
//! Garde-fou du rendu des pages : un `GET` de l'interface qui ne repond pas dans un delai borne est abandonne et relance.
//!
//! Pourquoi : avec Leptos 0.8.23 (derniere version stable), le rendu serveur d'une page peut ne jamais se terminer quand
//! plusieurs pages sont rendues en meme temps juste apres le demarrage (1 a 5 % des demarrages dans `tools/stress-cold-render.sh`,
//! y compris sur la page d'accueil ; tous les fils sont alors endormis : une attente n'est jamais reveillee, ce n'est ni
//! une boucle ni un verrou tenu). Le navigateur attendrait alors indefiniment. Un nouveau rendu de la meme requete
//! aboutit presque toujours, et un `GET` ne modifie rien : on peut le relancer sans risque.
//!
//! Portee : seulement `GET`/`HEAD` sans corps. Les fonctions serveur (`POST`) ne sont ni minutees ni relancees. Le rendu
//! abandonne laisse des taches orphelines qui paniquent plus tard en lisant un signal libere (« reactive value that has
//! already been disposed ») : bruyant dans le journal, sans effet sur les autres requetes (verifie par le test de charge).
//!
//! A retirer quand Leptos corrige le defaut : relancer `tools/stress-cold-render.sh` sans ce garde-fou (voir ADR 0021).

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, Response, StatusCode, header};
use tower::{Layer, Service};

/// Delai accorde a un rendu avant de le relancer (un rendu normal prend quelques dizaines de millisecondes).
pub const STALL_AFTER: Duration = Duration::from_secs(5);
/// Nombre total d'essais (le premier compris).
pub const ATTEMPTS: u32 = 3;

#[derive(Clone, Copy, Debug)]
pub struct RenderGuardLayer {
    stall_after: Duration,
    attempts: u32,
}

impl RenderGuardLayer {
    #[must_use]
    pub fn new() -> Self {
        Self::with(STALL_AFTER, ATTEMPTS)
    }

    #[must_use]
    pub fn with(stall_after: Duration, attempts: u32) -> Self {
        Self {
            stall_after,
            attempts: attempts.max(1),
        }
    }
}

impl Default for RenderGuardLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> Layer<S> for RenderGuardLayer {
    type Service = RenderGuard<S>;

    fn layer(&self, inner: S) -> RenderGuard<S> {
        RenderGuard {
            inner,
            stall_after: self.stall_after,
            attempts: self.attempts,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RenderGuard<S> {
    inner: S,
    stall_after: Duration,
    attempts: u32,
}

/// Une requete sans corps qu'on peut rejouer : `GET`/`HEAD` dont le corps est vide.
fn replayable(req: &Request<Body>) -> bool {
    matches!(*req.method(), Method::GET | Method::HEAD)
        && axum::body::HttpBody::is_end_stream(req.body())
}

/// Copie d'une requete sans corps (meme methode, adresse, en-tetes et extensions : session, adresse du pair...).
fn duplicate(req: &Request<Body>) -> Request<Body> {
    let mut copy = Request::new(Body::empty());
    *copy.method_mut() = req.method().clone();
    *copy.uri_mut() = req.uri().clone();
    *copy.version_mut() = req.version();
    *copy.headers_mut() = req.headers().clone();
    *copy.extensions_mut() = req.extensions().clone();
    copy
}

impl<S> Service<Request<Body>> for RenderGuard<S>
where
    S: Service<Request<Body>, Response = Response<Body>, Error = Infallible>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    type Response = Response<Body>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response<Body>, Infallible>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        // Le service pret est celui qu'on a verifie : on le garde, et on laisse un clone dans `self`.
        let ready = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, ready);
        let (stall_after, attempts) = (self.stall_after, self.attempts);
        Box::pin(async move {
            if !replayable(&req) {
                return inner.call(req).await;
            }
            let path = req.uri().path().to_string();
            for attempt in 1..=attempts {
                let mut service = inner.clone();
                match tokio::time::timeout(stall_after, service.call(duplicate(&req))).await {
                    Ok(response) => return response,
                    Err(_) if attempt < attempts => {
                        tracing::warn!(%path, attempt, "page render without answer: abandoned and retried");
                    }
                    Err(_) => {
                        tracing::error!(%path, attempts, "page render without answer after several tries");
                    }
                }
            }
            let mut unavailable = Response::new(Body::from("render unavailable, retry"));
            *unavailable.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
            unavailable
                .headers_mut()
                .insert(header::RETRY_AFTER, header::HeaderValue::from_static("1"));
            Ok(unavailable)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use axum::Router;
    use axum::routing::{get, post};
    use tower::ServiceExt;

    use super::*;

    const FAST: Duration = Duration::from_millis(80);

    /// Un routeur dont `/page` ne repond jamais aux `hang_first` premiers appels, puis repond 200.
    fn app(hang_first: u32, calls: &Arc<AtomicU32>) -> RenderGuard<Router> {
        let c = Arc::clone(calls);
        let router = Router::new()
            .route(
                "/page",
                get(move |headers: axum::http::HeaderMap| {
                    let c = Arc::clone(&c);
                    async move {
                        let n = c.fetch_add(1, Ordering::SeqCst) + 1;
                        if n <= hang_first {
                            std::future::pending::<()>().await;
                        }
                        format!(
                            "ok n={n} x={}",
                            headers
                                .get("x-test")
                                .and_then(|v| v.to_str().ok())
                                .unwrap_or("-")
                        )
                    }
                }),
            )
            .route(
                "/act",
                post({
                    let c = Arc::clone(calls);
                    move || {
                        let c = Arc::clone(&c);
                        async move {
                            c.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(250)).await;
                            "done"
                        }
                    }
                }),
            );
        RenderGuardLayer::with(FAST, 3).layer(router)
    }

    async fn body(r: Response<Body>) -> String {
        let bytes = http_body_util::BodyExt::collect(r.into_body())
            .await
            .unwrap()
            .to_bytes();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    fn get_req(path: &str) -> Request<Body> {
        Request::builder()
            .uri(path)
            .header("x-test", "garde")
            .body(Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn a_healthy_render_is_passed_through_once() {
        let calls = Arc::new(AtomicU32::new(0));
        let r = app(0, &calls).oneshot(get_req("/page")).await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(body(r).await, "ok n=1 x=garde");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_stalled_render_is_abandoned_and_retried_with_the_same_request() {
        let calls = Arc::new(AtomicU32::new(0));
        let r = app(2, &calls).oneshot(get_req("/page")).await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(
            body(r).await,
            "ok n=3 x=garde",
            "troisieme essai, en-tetes conserves"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn after_all_attempts_the_answer_is_503_with_retry_after() {
        let calls = Arc::new(AtomicU32::new(0));
        let r = app(99, &calls).oneshot(get_req("/page")).await.unwrap();
        assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(r.headers()[header::RETRY_AFTER], "1");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            3,
            "pas plus d'essais que prevu"
        );
    }

    #[tokio::test]
    async fn requests_that_change_things_are_neither_timed_out_nor_replayed() {
        let calls = Arc::new(AtomicU32::new(0));
        let req = Request::builder()
            .method(Method::POST)
            .uri("/act")
            .body(Body::empty())
            .unwrap();
        // 250 ms > delai du garde-fou (80 ms) : un POST lent doit aller au bout, une seule fois.
        let r = app(0, &calls).oneshot(req).await.unwrap();
        assert_eq!(body(r).await, "done");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_get_with_a_body_is_not_replayed() {
        let calls = Arc::new(AtomicU32::new(0));
        let req = Request::builder()
            .uri("/page")
            .body(Body::from("surprise"))
            .unwrap();
        let r = tokio::time::timeout(Duration::from_millis(400), app(1, &calls).oneshot(req)).await;
        assert!(
            r.is_err(),
            "sans relance, le rendu bloque reste bloque (pas de garde-fou sur un corps non rejouable)"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
