//! Simple IP-based rate limiting.

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Extension,
};
use dashmap::DashMap;
use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

/// Rate limiter state.
#[derive(Clone)]
pub struct RateLimiter {
    /// Map of IP -> (request count, window start)
    requests: Arc<DashMap<String, (u32, Instant)>>,
    /// Max requests per window
    max_requests: u32,
    /// Window duration
    window: Duration,
}

impl RateLimiter {
    pub fn new(max_requests: u32, window_secs: u64) -> Self {
        Self {
            requests: Arc::new(DashMap::new()),
            max_requests,
            window: Duration::from_secs(window_secs),
        }
    }

    /// Check if request should be allowed. Returns true if allowed.
    pub fn check(&self, ip: &str) -> bool {
        let now = Instant::now();

        let mut entry = self.requests.entry(ip.to_string()).or_insert((0, now));
        let (count, window_start) = entry.value_mut();

        // Reset if window expired
        if now.duration_since(*window_start) > self.window {
            *count = 0;
            *window_start = now;
        }

        // Check limit
        if *count >= self.max_requests {
            return false;
        }

        *count += 1;
        true
    }

    /// Periodically clean up old entries.
    pub fn cleanup(&self) {
        let now = Instant::now();
        self.requests
            .retain(|_, (_, start)| now.duration_since(*start) <= self.window * 2);
    }
}

/// Rate limiting middleware.
pub async fn rate_limit_middleware(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Extension(limiter): Extension<RateLimiter>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let ip = addr.ip().to_string();

    if !limiter.check(&ip) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "Rate limit exceeded. Try again later.",
        )
            .into_response();
    }

    next.run(request).await
}
