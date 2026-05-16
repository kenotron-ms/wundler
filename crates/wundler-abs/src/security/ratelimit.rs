//! Per-IP, in-process rate limiter for `POST /manifest`.

use std::net::IpAddr;
use std::num::NonZeroU32;
use std::sync::Arc;

use governor::{
    clock::DefaultClock, state::keyed::DefaultKeyedStateStore, Quota, RateLimiter,
};

/// Per-IP rate limiter type alias.
pub type IpRateLimiter = RateLimiter<IpAddr, DefaultKeyedStateStore<IpAddr>, DefaultClock>;

/// Build a rate limiter: `rate` rps per IP, `burst` request burst budget.
pub fn build_limiter(rate: NonZeroU32, burst: NonZeroU32) -> Arc<IpRateLimiter> {
    let quota = Quota::per_second(rate).allow_burst(burst);
    Arc::new(RateLimiter::keyed(quota))
}
