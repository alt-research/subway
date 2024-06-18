use async_trait::async_trait;
use jsonrpsee::core::JsonValue;
use jsonrpsee::types::ErrorObjectOwned;
use opentelemetry::trace::FutureExt;
use serde_json::value::RawValue;

use crate::extensions::whitelist::{WhiteAddress, Whitelist, ETH_CALL, SEND_RAW};
use crate::{
    middlewares::{CallRequest, CallResult, Middleware, MiddlewareBuilder, NextFn, RpcMethod, TRACER},
    utils::{TypeRegistry, TypeRegistryRef},
};

/// The related address is banned.
pub const ADDRESS_IS_BANNED: i32 = -33000;

/// The address must be parsed from the rpc parameters.
pub const UNKNOWN_ADDRESS: i32 = -33001;

/// This whitelist middleware should be used at the top level whenever possible.
pub struct WhitelistMiddleware {
    rpc_type: RpcType,
    addresses: Vec<WhiteAddress>,
}

#[derive(Debug, Eq, PartialEq)]
enum RpcType {
    EthCall,
    SendRaw,
}

impl WhitelistMiddleware {
    /// Return true if it's in whitelist.
    pub fn satisfy(&self, from: String, to: String) -> bool {
        let mut allowed = false;
        for address in &self.addresses {
            // if satisfy address from/to
            if address.satisfy(from.as_str(), to.as_str()) {
                allowed = true;
                break;
            }
        }

        allowed
    }
}

#[async_trait]
impl MiddlewareBuilder<RpcMethod, CallRequest, CallResult> for WhitelistMiddleware {
    async fn build(
        method: &RpcMethod,
        extensions: &TypeRegistryRef,
    ) -> Option<Box<dyn Middleware<CallRequest, CallResult>>> {
        let whitelist = extensions
            .read()
            .await
            .get::<Whitelist>()
            .expect("WhitelistConfig extension not found");

        if &method.method == SEND_RAW {
            Some(Box::new(Self {
                rpc_type: RpcType::SendRaw,
                addresses: whitelist.config.raw_tx_whitelist.clone(),
            }))
        } else if &method.method == ETH_CALL {
            Some(Box::new(Self {
                rpc_type: RpcType::EthCall,
                addresses: whitelist.config.eth_call_whitelist.clone(),
            }))
        } else {
            // other rpc types will skip this middleware.
            None
        }
    }
}

/// Extract the address from parameters and convert it into lowercase.
pub fn extract_address_from_to(params: &[JsonValue]) -> Result<(String, String), ErrorObjectOwned> {
    let p1 = params.get(0).ok_or_else(|| {
        ErrorObjectOwned::borrowed(
            UNKNOWN_ADDRESS,
            "Could not get the first param from rpc parameter",
            None,
        )
    })?;

    let from = p1.get("from").ok_or_else(|| {
        ErrorObjectOwned::borrowed(UNKNOWN_ADDRESS, "Could not get the `from` from rpc parameter", None)
    })?;
    let from: String = serde_json::from_value(from.clone()).map_err(|_err| {
        ErrorObjectOwned::borrowed(UNKNOWN_ADDRESS, "Could not parse `from` from rpc parameter", None)
    })?;

    let to = p1.get("to").ok_or_else(|| {
        ErrorObjectOwned::borrowed(UNKNOWN_ADDRESS, "Could not get the `to` from rpc parameter", None)
    })?;
    let to: String = serde_json::from_value(to.clone())
        .map_err(|_err| ErrorObjectOwned::borrowed(UNKNOWN_ADDRESS, "Could not parse `to` from rpc parameter", None))?;

    Ok((from.to_lowercase(), to.to_lowercase()))
}

pub fn banned_address_call_result(data: Option<&'static RawValue>) -> CallResult {
    Err(ErrorObjectOwned::borrowed(
        ADDRESS_IS_BANNED,
        "The address related to the rpc is banned",
        data,
    ))
}

#[async_trait]
impl Middleware<CallRequest, CallResult> for WhitelistMiddleware {
    async fn call(
        &self,
        request: CallRequest,
        context: TypeRegistry,
        next: NextFn<CallRequest, CallResult>,
    ) -> CallResult {
        async move {
            match self.rpc_type {
                RpcType::EthCall => {
                    let (from, to) = extract_address_from_to(&request.params)?;
                    if !self.satisfy(from, to) {
                        return banned_address_call_result(None);
                    }
                }
                RpcType::SendRaw => {
                    // TODO:
                }
            }

            next(request, context).await
        }
        .with_context(TRACER.context("whitelist"))
        .await
    }
}
