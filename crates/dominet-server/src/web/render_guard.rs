// SPDX-License-Identifier: AGPL-3.0-or-later
//! Page-rendering safeguard: an interface `GET` that does not answer within a bounded delay is abandoned and retried.
//!
//! Why: with Leptos 0.8.23 (latest stable version), the server rendering of a page may never finish when
//! several pages are rendered at the same time just after start-up (1 to 5% of the start-ups in `tools/stress-cold-render.sh`,
//! including on the home page; all the threads are then asleep: a wait is never woken up, it is neither
//! a loop nor a held lock). The browser would then wait forever. A new render of the same request
//! almost always succeeds, and a `GET` modifies nothing: it can be retried safely.
//!
//! Scope: only `GET`/`HEAD` without a body. Server functions (`POST`) are neither timed nor retried. The abandoned render
//! leaves orphan tasks that panic later when reading a disposed signal ("reactive value that has
//! already been disposed"): noisy in the log, without effect on the other requests (checked by the load test).
//!
//! To be removed when Leptos fixes the defect: rerun `tools/stress-cold-render.sh` without this safeguard (see ADR 0021).

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, Response, StatusCode, header};
use tower::{Layer, Service};

/// Delay granted to a render before retrying it (a normal render takes a few tens of milliseconds).
pub const STALL_AFTER: Duration = Duration::from_secs(5);
/// Total number of attempts (the first one included).
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

/// A request without a body that can be replayed: `GET`/`HEAD` whose body is empty.
fn replayable(req: &Request<Body>) -> bool {
    matches!(*req.method(), Method::GET | Method::HEAD)
        && axum::body::HttpBody::is_end_stream(req.body())
}

/// Copy of a request without a body (same method, address, headers and extensions: session, peer address...).
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
        // The ready service is the one we checked: we keep it, and leave a clone in `self`.
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

    /// A router whose `/page` never answers the first `hang_first` calls, then answers 200.
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
        // 250 ms > the safeguard's delay (80 ms): a slow POST must run to completion, exactly once.
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
