//! Per-IP, in-process rate limiter for `POST /manifest`.

use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU32;
use std::sync::Arc;

use axum::{
    extract::{ConnectInfo, Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use governor::{
    clock::DefaultClock, state::keyed::DefaultKeyedStateStore, Quota, RateLimiter,
};
use serde_json::json;

// ---------------------------------------------------------------------------
// Type alias
// ---------------------------------------------------------------------------

pub type IpRateLimiter = RateLimiter<IpAddr, DefaultKeyedStateStore<IpAddr>, DefaultClock>;

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

pub fn build_limiter(rate: NonZeroU32, burst: NonZeroU32) -> Arc<IpRateLimiter> {
    let quota = Quota::per_second(rate).allow_burst(burst);
    Arc::new(RateLimiter::keyed(quota))
}

// ---------------------------------------------------------------------------
// Middleware
// ---------------------------------------------------------------------------

/// Axum middleware that throttles requests per source IP.
///
/// No `ConnectInfo<SocketAddr>` extension → pass through (test ergonomics).
/// Otherwise check the limiter; deny with 429 + JSON body on exhaustion.
pub async fn rate_limit_mw(
    State(lim): State<Arc<IpRateLimiter>>,
    req: Request,
    next: Next,
) -> Response {
    let ip: Option<IpAddr> = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip());

    let Some(ip) = ip else {
        return next.run(req).await;
    };

    match lim.check_key(&ip) {
        Ok(()) => next.run(req).await,
        Err(_negative) => too_many_requests(),
    }
}

fn too_many_requests() -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        Json(json!({ "error": "rate limit exceeded" })),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use axum::{
        body::Body,
        extract::ConnectInfo,
        http::{Request, StatusCode},
        middleware,
        routing::get,
        Router,
    };
    use tower::ServiceExt; // for `oneshot`

    /// Build a tiny app with an outer `inject_ip` layer that optionally seeds
    /// `ConnectInfo<SocketAddr>` into request extensions, then `rate_limit_mw`
    /// runs against a trivial `GET /` handler.
    fn app(limiter: Arc<IpRateLimiter>, ip: Option<IpAddr>) -> Router {
        let router = Router::new()
            .route("/", get(|| async { "ok" }))
            .layer(middleware::from_fn_with_state(limiter, rate_limit_mw));

        match ip {
            None => router,
            Some(ip) => router.layer(middleware::from_fn(
                move |mut req: Request<Body>, next: middleware::Next| {
                    let ip = ip;
                    async move {
                        let ci: ConnectInfo<SocketAddr> =
                            ConnectInfo(SocketAddr::new(ip, 49_152));
                        req.extensions_mut().insert(ci);
                        next.run(req).await
                    }
                },
            )),
        }
    }

    async fn hit(app: &Router) -> StatusCode {
        let resp = app
            .clone()
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        resp.status()
    }

    #[tokio::test]
    async fn skips_when_no_connect_info() {
        // 1 rps, burst 1 — would be very easy to trip if the limiter ran.
        let lim = build_limiter(
            NonZeroU32::new(1).unwrap(),
            NonZeroU32::new(1).unwrap(),
        );
        let app = app(lim, None);

        for _ in 0..20 {
            assert_eq!(hit(&app).await, StatusCode::OK);
        }
    }

    #[tokio::test]
    async fn blocks_when_quota_exceeded_for_same_ip() {
        let lim = build_limiter(
            NonZeroU32::new(1).unwrap(),
            NonZeroU32::new(1).unwrap(),
        );
        let app = app(lim, Some(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));

        assert_eq!(hit(&app).await, StatusCode::OK);

        let mut denied = 0;
        for _ in 0..10 {
            if hit(&app).await == StatusCode::TOO_MANY_REQUESTS {
                denied += 1;
            }
        }
        assert!(denied >= 5, "expected ≥5 denials within burst window, got {denied}");
    }

    #[tokio::test]
    async fn allows_burst_then_blocks() {
        let lim = build_limiter(
            NonZeroU32::new(1).unwrap(),
            NonZeroU32::new(5).unwrap(),
        );
        let app = app(lim, Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));

        for i in 0..5 {
            assert_eq!(
                hit(&app).await,
                StatusCode::OK,
                "burst request {i} must succeed"
            );
        }
        assert_eq!(hit(&app).await, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn different_ips_have_independent_buckets() {
        let lim = build_limiter(
            NonZeroU32::new(1).unwrap(),
            NonZeroU32::new(1).unwrap(),
        );
        let app_a = app(lim.clone(), Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        let app_b = app(lim, Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2))));

        assert_eq!(hit(&app_a).await, StatusCode::OK);
        assert_eq!(hit(&app_b).await, StatusCode::OK);
        assert_eq!(hit(&app_a).await, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(hit(&app_b).await, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn body_is_json_error_on_429() {
        let lim = build_limiter(
            NonZeroU32::new(1).unwrap(),
            NonZeroU32::new(1).unwrap(),
        );
        let app = app(lim, Some(IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1))));

        let _ = hit(&app).await;

        let resp = app
            .clone()
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

        let bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes)
            .expect("429 body must be JSON");
        assert_eq!(json["error"], "rate limit exceeded");
    }
}
