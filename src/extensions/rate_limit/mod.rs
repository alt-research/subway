use super::{Extension, ExtensionRegistry};
use governor::{DefaultDirectRateLimiter, DefaultKeyedRateLimiter, Jitter, Quota, RateLimiter};
use serde::Deserialize;
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::{sync::Arc, time::Duration};

mod connection;
mod global;
mod ip;
mod method;
mod weight;
mod xff;

use crate::extensions::rate_limit::global::GlobalRateLimitLayer;
use crate::extensions::rate_limit::method::MethodRateLimitLayer;
pub use connection::{ConnectionRateLimit, ConnectionRateLimitLayer};
pub use ip::{IpRateLimit, IpRateLimitLayer};
pub use weight::MethodWeights;
pub use xff::XFF;

#[derive(Deserialize, Debug, Clone, Default)]
pub struct RateLimitConfig {
    pub ip: Option<Rule>,
    pub methods: Option<MethodsRule>,
    pub connection: Option<Rule>,
    pub global: Option<Rule>,
    #[serde(default)]
    pub use_xff: bool,
}

/// The rule for every method.
///
/// Only work for method rate limit.
#[derive(Deserialize, Debug, Clone, Default)]
pub struct MethodsRule {
    /// The burst for every rpc.
    pub bursts: HashMap<String, u32>,
    /// Return the responses with delay instead of returning a rate limit jsonrpc error directly if true.
    #[serde(default)]
    pub blocking: bool,
    /// period is the period of time in which the burst is allowed
    #[serde(default = "default_period_secs")]
    pub period_secs: u64,
    // jitter_millis is the maximum amount of jitter to add to the rate limit
    // this is to prevent a thundering herd problem https://en.wikipedia.org/wiki/Thundering_herd_problem
    // e.g. if jitter_up_to_millis is 1000, then additional delay of random(0, 1000) milliseconds will be added
    #[serde(default = "default_jitter_up_to_millis")]
    pub jitter_up_to_millis: u64,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct Rule {
    /// burst is the maximum number of requests that can be made in a period
    pub burst: u32,
    /// period is the period of time in which the burst is allowed
    #[serde(default = "default_period_secs")]
    pub period_secs: u64,
    // jitter_millis is the maximum amount of jitter to add to the rate limit
    // this is to prevent a thundering herd problem https://en.wikipedia.org/wiki/Thundering_herd_problem
    // e.g. if jitter_up_to_millis is 1000, then additional delay of random(0, 1000) milliseconds will be added
    #[serde(default = "default_jitter_up_to_millis")]
    pub jitter_up_to_millis: u64,
    /// Return the responses with delay instead of returning a rate limit jsonrpc error directly if true.
    #[serde(default)]
    pub blocking: bool,
}

fn default_period_secs() -> u64 {
    1
}

fn default_jitter_up_to_millis() -> u64 {
    100
}

pub struct RateLimitBuilder {
    config: RateLimitConfig,

    ip_jitter: Option<Jitter>,
    ip_limiter: Option<Arc<DefaultKeyedRateLimiter<String>>>,
    ip_blocking: bool,

    global_jitter: Option<Jitter>,
    global_limiter: Option<Arc<DefaultDirectRateLimiter>>,
    global_blocking: bool,

    method_jitter: Option<Jitter>,
    method_blocking: bool,
    method_limiters: HashMap<String, Arc<DefaultDirectRateLimiter>>,
}

#[async_trait::async_trait]
impl Extension for RateLimitBuilder {
    type Config = RateLimitConfig;

    async fn from_config(config: &Self::Config, _registry: &ExtensionRegistry) -> Result<Self, anyhow::Error> {
        Ok(Self::new(config.clone()))
    }
}

impl RateLimitBuilder {
    pub fn new(config: RateLimitConfig) -> Self {
        // make sure all rules are valid
        if let Some(ref rule) = config.ip {
            assert!(rule.burst > 0, "burst must be greater than 0");
            assert!(rule.period_secs > 0, "period_secs must be greater than 0");
        }
        if let Some(ref rule) = config.connection {
            assert!(rule.burst > 0, "burst must be greater than 0");
            assert!(rule.period_secs > 0, "period_secs must be greater than 0");
        }

        let mut ip_limiter = None;
        let mut ip_jitter = None;
        let mut ip_blocking = false;
        if let Some(ref rule) = config.ip {
            let burst = NonZeroU32::new(rule.burst).unwrap();
            let quota = build_quota(burst, Duration::from_secs(rule.period_secs));
            ip_limiter = Some(Arc::new(RateLimiter::keyed(quota)));
            ip_jitter = Some(Jitter::up_to(Duration::from_millis(rule.jitter_up_to_millis)));
            ip_blocking = rule.blocking;
        }

        let mut global_limiter = None;
        let mut global_jitter = None;
        let mut global_blocking = false;
        if let Some(ref rule) = config.global {
            let burst = NonZeroU32::new(rule.burst).unwrap();
            let quota = build_quota(burst, Duration::from_secs(rule.period_secs));
            global_limiter = Some(Arc::new(DefaultDirectRateLimiter::direct(quota)));
            global_jitter = Some(Jitter::up_to(Duration::from_millis(rule.jitter_up_to_millis)));
            global_blocking = rule.blocking;
        }

        let mut method_jitter = None;
        let mut method_blocking = false;
        let mut method_limiters = HashMap::new();
        if let Some(ref methods_rule) = config.methods {
            for (method, burst) in methods_rule.bursts.iter() {
                let burst = NonZeroU32::new(*burst).unwrap();
                let quota = build_quota(burst, Duration::from_secs(methods_rule.period_secs));
                let method_limiter = Arc::new(DefaultDirectRateLimiter::direct(quota));
                method_limiters.insert(method.clone(), method_limiter);
            }
            method_jitter = Some(Jitter::up_to(Duration::from_millis(methods_rule.jitter_up_to_millis)));
            method_blocking = methods_rule.blocking;
        }

        Self {
            config,

            ip_jitter,
            ip_limiter,
            ip_blocking,

            global_jitter,
            global_limiter,
            global_blocking,

            method_jitter,
            method_limiters,
            method_blocking,
        }
    }

    pub fn connection_limit(&self, method_weights: MethodWeights) -> Option<ConnectionRateLimitLayer> {
        if let Some(ref rule) = self.config.connection {
            let burst = NonZeroU32::new(rule.burst).unwrap();
            let period = Duration::from_secs(rule.period_secs);
            let jitter = Jitter::up_to(Duration::from_millis(rule.jitter_up_to_millis));
            Some(ConnectionRateLimitLayer::new(burst, period, jitter, method_weights).blocking(rule.blocking))
        } else {
            None
        }
    }

    pub fn ip_limit(&self, remote_ip: String, method_weights: MethodWeights) -> Option<IpRateLimitLayer> {
        self.ip_limiter.as_ref().map(|limiter| {
            IpRateLimitLayer::new(
                remote_ip,
                limiter.clone(),
                self.ip_jitter.unwrap_or_default(),
                method_weights,
            )
            .blocking(self.ip_blocking)
        })
    }

    pub fn method_limits(&self, method_weights: MethodWeights) -> Option<MethodRateLimitLayer> {
        MethodRateLimitLayer::new(
            self.method_limiters.clone(),
            self.method_jitter.unwrap_or_default(),
            method_weights,
        )
        .map(|layer| layer.blocking(self.method_blocking))
    }

    pub fn global_limit(&self, method_weights: MethodWeights) -> Option<GlobalRateLimitLayer> {
        self.global_limiter.as_ref().map(|limiter| {
            GlobalRateLimitLayer::new(limiter.clone(), self.global_jitter.unwrap_or_default(), method_weights)
                .blocking(self.global_blocking)
        })
    }

    // whether to use the X-Forwarded-For header to get the remote ip
    pub fn use_xff(&self) -> bool {
        self.config.use_xff
    }
}

pub fn build_quota(burst: NonZeroU32, period: Duration) -> Quota {
    let replenish_interval_ns = period.as_nanos() / (burst.get() as u128);
    Quota::with_period(Duration::from_nanos(replenish_interval_ns as u64))
        .unwrap()
        .allow_burst(burst)
}
