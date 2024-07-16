use std::{num::NonZeroUsize, sync::Arc, time::Duration};

use async_trait::async_trait;
use blake2::Blake2b512;
use futures::future::FutureExt as _;
use jsonrpsee::core::JsonValue;
use opentelemetry::trace::get_active_span;
use opentelemetry::{
    trace::{FutureExt, Span, TraceContextExt},
    Context, KeyValue,
};
use opentelemetry_semantic_conventions as semconv;

use crate::{
    config::BlockCacheParams,
    extensions::{
        api::EthApi,
        cache::BlockCache,
        prometheus::{get_rpc_metrics, RpcMetrics},
    },
    middlewares::{CallRequest, CallResult, Middleware, MiddlewareBuilder, NextFn, RpcMethod, TRACER},
    utils::{Cache, CacheKey, TypeRegistry, TypeRegistryRef},
};

pub struct BlockCacheMiddleware {
    api: Arc<EthApi>,
    index: usize,

    finalized_cache: Cache<Blake2b512>,
    recent_cache: Cache<Blake2b512>,

    metrics: RpcMetrics,
}

impl BlockCacheMiddleware {
    pub fn new(
        api: Arc<EthApi>,
        index: usize,
        finalized_cache: Cache<Blake2b512>,
        recent_cache: Cache<Blake2b512>,
        metrics: RpcMetrics,
    ) -> Self {
        Self {
            api,
            index,
            finalized_cache,
            recent_cache,
            metrics,
        }
    }
}

#[async_trait]
impl MiddlewareBuilder<RpcMethod, CallRequest, CallResult> for BlockCacheMiddleware {
    async fn build(
        method: &RpcMethod,
        extensions: &TypeRegistryRef,
    ) -> Option<Box<dyn Middleware<CallRequest, CallResult>>> {
        let index = method.params.iter().position(|p| p.ty == "BlockTag" && p.inject)?;

        let eth_api = extensions
            .read()
            .await
            .get::<EthApi>()
            .expect("EthApi extension not found");

        let cache = extensions
            .read()
            .await
            .get::<BlockCache>()
            .expect("Cache extension not found");

        let metrics = get_rpc_metrics(extensions).await;

        // used by finalized blocks
        let finalized_cache = {
            let size = match &method.block_cache {
                // do not cache if size is 0, otherwise use default size
                Some(BlockCacheParams {
                    finalized_size: Some(0),
                    recent_size: Some(0),
                    ..
                }) => return None,
                Some(BlockCacheParams { finalized_size, .. }) => {
                    finalized_size.unwrap_or(cache.config.default_finalized_size)
                }
                None => cache.config.default_finalized_size,
            };
            let ttl_seconds = match &method.block_cache {
                // ttl zero means cache forever
                Some(params) => params
                    .finalized_ttl_seconds(cache.config.ttl_unit_seconds, cache.config.default_finalized_ttl_units),
                None => cache.config.default_finalized_ttl_seconds(),
            };
            Cache::new(NonZeroUsize::new(size)?, ttl_seconds.map(Duration::from_secs))
        };

        // used by recent blocks (block range: finalized ~ latest), it should be a short-term cache.
        let recent_cache = {
            let size = match method.block_cache {
                Some(BlockCacheParams { recent_size, .. }) => recent_size.unwrap_or(cache.config.default_recent_size),
                None => cache.config.default_recent_size,
            };
            let ttl_seconds = match &method.block_cache {
                Some(params) => {
                    params.recent_ttl_seconds(cache.config.ttl_unit_seconds, cache.config.default_recent_ttl_units)
                }
                None => cache.config.default_recent_ttl_seconds(),
            };
            Cache::new(NonZeroUsize::new(size)?, ttl_seconds.map(Duration::from_secs))
        };

        Some(Box::new(BlockCacheMiddleware::new(
            eth_api,
            index,
            finalized_cache,
            recent_cache,
            metrics,
        )))
    }
}

#[async_trait]
impl Middleware<CallRequest, CallResult> for BlockCacheMiddleware {
    async fn call(
        &self,
        request: CallRequest,
        context: TypeRegistry,
        next: NextFn<CallRequest, CallResult>,
    ) -> CallResult {
        let mut span = TRACER.span("block_cache");

        let request_method = request.method.clone();
        let request_params = serde_json::to_string(&request.params).expect("serialize JSON value shouldn't be fail");
        span.set_attributes([
            KeyValue::new(semconv::resource::RPC_METHOD, request_method),
            KeyValue::new("rpc.params", request_params),
        ]);

        BlockCacheMiddlewareImpl::new(self)
            .call(request, context, next)
            .with_context(Context::current_with_span(span))
            .await
    }
}

#[derive(PartialEq, Default)]
enum CacheAction {
    Bypass,
    Recent,
    #[default]
    Finalized,
}

impl CacheAction {
    fn should_cache(&self) -> bool {
        matches!(self, Self::Recent | Self::Finalized)
    }
}

struct BlockCacheMiddlewareImpl {
    api: Arc<EthApi>,
    index: usize,

    cache_action: CacheAction,
    recent_cache: Cache<Blake2b512>,
    finalized_cache: Cache<Blake2b512>,

    metrics: RpcMetrics,
}

impl BlockCacheMiddlewareImpl {
    fn new(middleware: &BlockCacheMiddleware) -> Self {
        Self {
            api: middleware.api.clone(),
            index: middleware.index,

            cache_action: Default::default(),
            recent_cache: middleware.recent_cache.clone(),
            finalized_cache: middleware.finalized_cache.clone(),

            metrics: middleware.metrics.clone(),
        }
    }

    async fn replace_block_tag(&mut self, request: &CallRequest) -> Option<JsonValue> {
        let param = request.params.get(self.index)?;
        if !param.is_string() {
            return None;
        }

        match param.as_str().unwrap_or_default() {
            // "earliest" is always going to be genesis, so we don't need to replace it with
            // specified block number
            "earliest" => None,
            // Actually, "safe" block should be cached, but we need to add a new background task to
            // poll and get the current safe block, so we won't cache it yet.
            "pending" | "safe" => {
                self.cache_action = CacheAction::Bypass;
                None
            }
            "latest" => {
                self.cache_action = CacheAction::Recent;
                let (_, number) = self.api.get_head().read().await;
                Some(format!("0x{:x}", number).into())
            }
            "finalized" => {
                let finalized_head = self.api.current_finalized_head();
                if let Some((_, finalized_number)) = finalized_head {
                    Some(format!("0x{:x}", finalized_number).into())
                } else {
                    self.cache_action = CacheAction::Bypass;
                    None
                }
            }
            number => {
                let number = number
                    .strip_prefix("0x")
                    .and_then(|hex| u64::from_str_radix(hex, 16).ok());

                if let Some((_, finalized_number)) = self.api.current_finalized_head() {
                    if let Some(number) = number {
                        let (_, latest_number) = self.api.get_head().read().await;
                        if number > finalized_number && number <= latest_number {
                            self.cache_action = CacheAction::Recent;
                        } else if number > latest_number {
                            self.cache_action = CacheAction::Bypass;
                        }
                    }
                }

                if self.cache_action.should_cache() {
                    number.map(|n| format!("0x{:x}", n).into())
                } else {
                    None
                }
            }
        }
    }

    async fn replace(&mut self, mut request: CallRequest) -> CallRequest {
        let changed_param = self.replace_block_tag(&request).await;
        if let Some(param) = changed_param {
            tracing::trace!(
                "Replacing params {:?} updated with {:?}",
                request.params,
                (self.index, &param),
            );
            request.params.remove(self.index);
            request.params.insert(self.index, param);
        }
        request
    }

    async fn call(
        &mut self,
        request: CallRequest,
        context: TypeRegistry,
        next: NextFn<CallRequest, CallResult>,
    ) -> CallResult {
        let request = self.replace(request).await;

        let metrics = self.metrics.clone();

        match self.cache_action {
            CacheAction::Bypass => {
                get_active_span(|span| {
                    span.set_attributes([KeyValue::new("bypass", true), KeyValue::new("hit", false)])
                });
                next(request, context).await
            }
            CacheAction::Recent => {
                let key = CacheKey::<Blake2b512>::new(&request.method, &request.params);

                let mut hit = true;
                metrics.recent_cache_query(&request.method);

                let result = self
                    .recent_cache
                    .get_or_insert_with(key.clone(), || {
                        hit = false;
                        metrics.recent_cache_miss(&request.method);
                        next(request, context).boxed()
                    })
                    .await;

                get_active_span(|span| {
                    span.set_attributes([KeyValue::new("bypass", false), KeyValue::new("hit", hit)])
                });

                if let Ok(ref value) = result {
                    // avoid caching null value because it usually means data not available,
                    // but it could be available in the future
                    if value.is_null() {
                        self.recent_cache.remove(&key).await;
                    }
                }
                result
            }
            CacheAction::Finalized => {
                let key = CacheKey::<Blake2b512>::new(&request.method, &request.params);

                let mut hit = true;
                metrics.finalized_cache_query(&request.method);

                let result = self
                    .finalized_cache
                    .get_or_insert_with(key.clone(), || {
                        hit = false;
                        metrics.finalized_cache_miss(&request.method);
                        next(request, context).boxed()
                    })
                    .await;

                get_active_span(|span| {
                    span.set_attributes([KeyValue::new("bypass", false), KeyValue::new("hit", hit)])
                });

                if let Ok(ref value) = result {
                    // avoid caching null value because it usually means data not available,
                    // but it could be available in the future
                    if value.is_null() {
                        self.finalized_cache.remove(&key).await;
                    }
                }
                result
            }
        }
    }
}
