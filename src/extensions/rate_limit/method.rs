use crate::extensions::rate_limit::MethodWeights;
use crate::utils::errors;
use futures::{future::BoxFuture, FutureExt};
use governor::{DefaultDirectRateLimiter, Jitter};
use jsonrpsee::{
    server::{middleware::rpc::RpcServiceT, types::Request},
    MethodResponse,
};
use std::collections::HashMap;
use std::{num::NonZeroU32, sync::Arc};

#[derive(Clone)]
pub struct MethodRateLimitLayer {
    limiters: HashMap<String, Arc<DefaultDirectRateLimiter>>,
    jitter: Jitter,
    method_weights: MethodWeights,
    blocking: bool,
}

impl MethodRateLimitLayer {
    pub fn new(
        limiters: HashMap<String, Arc<DefaultDirectRateLimiter>>,
        jitter: Jitter,
        method_weights: MethodWeights,
    ) -> Option<Self> {
        if limiters.is_empty() {
            return None;
        }
        Some(Self {
            limiters,
            jitter,
            method_weights,
            blocking: false,
        })
    }

    pub fn blocking(mut self, blocking: bool) -> Self {
        self.blocking = blocking;
        self
    }
}

impl<S> tower::Layer<S> for MethodRateLimitLayer {
    type Service = MethodRateLimit<S>;

    fn layer(&self, service: S) -> Self::Service {
        MethodRateLimit::new(service, self.limiters.clone(), self.jitter, self.method_weights.clone())
            .blocking(self.blocking)
    }
}

#[derive(Clone)]
pub struct MethodRateLimit<S> {
    service: S,
    limiters: HashMap<String, Arc<DefaultDirectRateLimiter>>,
    jitter: Jitter,
    method_weights: MethodWeights,
    blocking: bool,
}

impl<S> MethodRateLimit<S> {
    pub fn new(
        service: S,
        limiters: HashMap<String, Arc<DefaultDirectRateLimiter>>,
        jitter: Jitter,
        method_weights: MethodWeights,
    ) -> Self {
        Self {
            service,
            limiters,
            jitter,
            method_weights,
            blocking: false,
        }
    }

    pub fn blocking(mut self, blocking: bool) -> Self {
        self.blocking = blocking;
        self
    }
}

impl<'a, S> RpcServiceT<'a> for MethodRateLimit<S>
where
    S: RpcServiceT<'a> + Send + Sync + Clone + 'static,
{
    type Future = BoxFuture<'a, MethodResponse>;

    fn call(&self, req: Request<'a>) -> Self::Future {
        let method_name = req.method_name();
        // no config for this method so no limit for it.
        let Some(limiter) = self.limiters.get(method_name).cloned() else {
            let service = self.service.clone();
            return async move { service.call(req).await }.boxed();
        };
        let jitter = self.jitter;
        let service = self.service.clone();
        let weight = self.method_weights.get(method_name);
        let blocking = self.blocking;

        async move {
            if let Some(n) = NonZeroU32::new(weight) {
                if blocking {
                    limiter
                        .until_n_ready_with_jitter(n, jitter)
                        .await
                        .expect("check_n have been done during init");
                } else {
                    match limiter.check_n(n).expect("check_n have been done during init") {
                        Ok(_) => {}
                        Err(_) => return MethodResponse::error(req.id, errors::reached_rate_limit()),
                    }
                }
            }
            service.call(req).await
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{RpcMethod, RpcSubscription, RpcSubscriptionMethod};
    use crate::extensions::rate_limit::build_quota;
    use governor::RateLimiter;
    use jsonrpsee::types::Id;
    use jsonrpsee::ResponsePayload;
    use std::time::Duration;

    #[derive(Clone)]
    struct MockService;
    impl RpcServiceT<'static> for MockService {
        type Future = BoxFuture<'static, MethodResponse>;

        fn call(&self, req: Request<'static>) -> Self::Future {
            async move { MethodResponse::response(req.id, ResponsePayload::success("ok"), 1024) }.boxed()
        }
    }

    #[tokio::test]
    async fn method_rate_limit_works() {
        let burst = NonZeroU32::new(10).unwrap();
        let period = Duration::from_millis(100);
        let quota = build_quota(burst, period);
        let mut limiters = HashMap::new();
        limiters.insert("test".to_string(), Arc::new(RateLimiter::direct(quota)));
        limiters.insert("subscribe".to_string(), Arc::new(RateLimiter::direct(quota)));
        let weights = MethodWeights::from_config(
            &[RpcMethod {
                method: "test".to_string(),
                rate_limit_weight: 10,
                ..Default::default()
            }],
            &[RpcSubscription {
                subscribe: RpcSubscriptionMethod {
                    method: "subscribe".to_string(),
                    rate_limit_weight: 10,
                },
                unsubscribe: RpcSubscriptionMethod {
                    method: "unsubscribe".to_string(),
                    rate_limit_weight: 10,
                },
                name: "subscribe".to_string(),
                merge_strategy: None,
            }],
        );

        let service = MethodRateLimit::new(MockService, limiters, Jitter::up_to(Duration::from_millis(1)), weights)
            .blocking(true);

        let batch = |method_name: String, service: MethodRateLimit<MockService>, count: usize, delay| async move {
            tokio::time::sleep(Duration::from_millis(delay)).await;
            let calls = (1..=count)
                .map(|id| service.call(Request::new(method_name.clone().into(), None, Id::Number(id as u64))))
                .collect::<Vec<_>>();
            let results = futures::future::join_all(calls).await;
            assert_eq!(results.iter().filter(|r| r.is_success()).count(), count);
        };

        // limited by 10 weights
        {
            let start = tokio::time::Instant::now();
            tokio::spawn(batch("test".to_string(), service.clone(), 1, 0))
                .await
                .unwrap();
            let duration = start.elapsed().as_millis();
            assert!(duration < 100, "actual duration: {}", duration);

            let start = tokio::time::Instant::now();
            tokio::spawn(batch("test".to_string(), service.clone(), 2, 0))
                .await
                .unwrap();
            let duration = start.elapsed().as_millis();
            assert!(duration > 100, "actual duration: {}", duration);

            let start = tokio::time::Instant::now();
            tokio::spawn(batch("subscribe".to_string(), service.clone(), 1, 0))
                .await
                .unwrap();
            let duration = start.elapsed().as_millis();
            assert!(duration < 100, "actual duration: {}", duration);

            let start = tokio::time::Instant::now();
            tokio::spawn(batch("subscribe".to_string(), service.clone(), 2, 0))
                .await
                .unwrap();
            let duration = start.elapsed().as_millis();
            assert!(duration > 100, "actual duration: {}", duration);
        }

        // limited by default 1 weight.
        {
            let start = tokio::time::Instant::now();
            tokio::spawn(batch("foo".to_string(), service.clone(), 5, 0))
                .await
                .unwrap();
            let duration = start.elapsed().as_millis();
            assert!(duration < 100, "actual duration: {}", duration);

            tokio::spawn(batch("foo".to_string(), service.clone(), 6, 0))
                .await
                .unwrap();
            let duration = start.elapsed().as_millis();
            assert!(duration < 100, "actual duration: {}", duration);
        }
    }
}
