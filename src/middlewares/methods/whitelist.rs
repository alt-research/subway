use async_trait::async_trait;
use jsonrpsee::core::JsonValue;
use jsonrpsee::types::ErrorObjectOwned;
use opentelemetry::trace::FutureExt;
use serde_json::value::RawValue;

use alloy_consensus::{Transaction, TxEip4844Variant, TxEnvelope};
use alloy_eips::eip2718::Decodable2718;
use alloy_primitives::{Address, TxKind};

use crate::extensions::whitelist::{WhiteAddress, Whitelist, ETH_CALL, SEND_RAW_TX, SEND_TX};
use crate::utils::ToAddress;
use crate::{
    middlewares::{CallRequest, CallResult, Middleware, MiddlewareBuilder, NextFn, RpcMethod, TRACER},
    utils::{TypeRegistry, TypeRegistryRef},
};

/// The related address is banned.
pub const ADDRESS_IS_BANNED: i32 = -33000;

/// The address must be parsed from the rpc parameters.
pub const UNKNOWN_ADDRESS: i32 = -33001;

///The transaction could not be decoded.
pub const ILLEGAL_TX: i32 = -33002;

/// The transaction signature is invalid.
pub const ILLEGAL_TX_SIGNATURE: i32 = -33003;

/// This whitelist middleware should be used at the top level whenever possible.
pub struct WhitelistMiddleware {
    rpc_type: RpcType,
    addresses: Vec<WhiteAddress>,
}

#[derive(Debug, Eq, PartialEq)]
enum RpcType {
    EthCall,
    SendRawTX,
    SendTX,
}

impl WhitelistMiddleware {
    /// Return true if it's in whitelist.
    pub fn satisfy(&self, from: &Address, to: &ToAddress) -> bool {
        let mut allowed = false;
        for address in &self.addresses {
            // if satisfy address from/to
            if address.satisfy(from, to) {
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

        match method.method.as_str() {
            ETH_CALL => Some(Box::new(Self {
                rpc_type: RpcType::EthCall,
                addresses: whitelist.config.eth_call_whitelist.clone(),
            })),
            SEND_TX => Some(Box::new(Self {
                rpc_type: RpcType::SendRawTX,
                addresses: whitelist.config.raw_tx_whitelist.clone(),
            })),
            SEND_RAW_TX => Some(Box::new(Self {
                rpc_type: RpcType::SendTX,
                addresses: whitelist.config.tx_whitelist.clone(),
            })),
            _ => {
                // other rpc types will skip this middleware.
                None
            }
        }
    }
}

/// Extract the address from `eth_call`/`eth_sendTranslation` parameters and convert it into lowercase.
pub fn extract_address_from_to(params: &[JsonValue]) -> Result<(Address, ToAddress), ErrorObjectOwned> {
    let p1 = params.get(0).ok_or_else(|| {
        ErrorObjectOwned::borrowed(
            UNKNOWN_ADDRESS,
            "Could not get the first param from rpc parameter",
            None,
        )
    })?;

    // `from` must exist.
    let from = p1.get("from").ok_or_else(|| {
        ErrorObjectOwned::borrowed(UNKNOWN_ADDRESS, "Could not get the `from` from rpc parameter", None)
    })?;
    let from: Address = serde_json::from_value(from.clone()).map_err(|_err| {
        ErrorObjectOwned::borrowed(UNKNOWN_ADDRESS, "Could not parse `from` from rpc parameter", None)
    })?;

    // When not get, it means `Create`.
    let to = p1.get("to");
    let to = if let Some(to) = to {
        serde_json::from_value(to.clone())
            .map_err(|_err| ErrorObjectOwned::borrowed(UNKNOWN_ADDRESS, "Could not parse `to` from rpc parameter", None))?
    } else {
        TxKind::Create
    };

    Ok((from, to.into()))
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
            // Not exist whitelist so return directly.
            if self.addresses.is_empty() {
                return next(request, context).await;
            }

            match self.rpc_type {
                RpcType::EthCall | RpcType::SendTX => {
                    let (from, to) = extract_address_from_to(&request.params)?;
                    if !self.satisfy(&from, &to) {
                        return banned_address_call_result(None);
                    }
                }
                RpcType::SendRawTX => {
                    let p1 = request.params.get(0).ok_or_else(|| {
                        ErrorObjectOwned::borrowed(
                            UNKNOWN_ADDRESS,
                            "Could not get the first param from rpc parameter",
                            None,
                        )
                    })?;
                    let rlp_hex: String = serde_json::from_value(p1.clone()).map_err(|_err| {
                        ErrorObjectOwned::borrowed(ILLEGAL_TX, "Could not get the first param from rpc parameter", None)
                    })?;
                    let tx: TxEnvelope = TxEnvelope::decode_2718(&mut rlp_hex.as_bytes())
                        .map_err(|_err| ErrorObjectOwned::borrowed(ILLEGAL_TX, "Could not decode the raw txn", None))?;
                    let from = extract_signer_from_tx_envelop(&tx)?;
                    let to = extract_to_address_from_tx_envelop(&tx);
                    if !self.satisfy(&from, &to) {
                        return banned_address_call_result(None);
                    }
                    // TODO:
                }
            }

            next(request, context).await
        }
        .with_context(TRACER.context("whitelist"))
        .await
    }
}

#[inline]
fn extract_signer_from_tx_envelop(tx: &TxEnvelope) -> Result<Address, ErrorObjectOwned> {
    let signer = match tx {
        TxEnvelope::Legacy(tx) => tx.recover_signer(),
        TxEnvelope::Eip2930(tx) => tx.recover_signer(),
        TxEnvelope::Eip1559(tx) => tx.recover_signer(),
        TxEnvelope::Eip4844(tx) => tx.recover_signer(),
        _ => {
            unreachable!()
        }
    }
    .map_err(|_err| ErrorObjectOwned::owned(ILLEGAL_TX_SIGNATURE, "Could not recover signer from tx", Some(tx)))?;

    Ok(signer)
}

#[inline]
fn extract_to_address_from_tx_envelop(tx: &TxEnvelope) -> ToAddress {
    let to = match tx {
        TxEnvelope::Legacy(tx) => tx.tx().to,
        TxEnvelope::Eip1559(tx) => tx.tx().to,
        TxEnvelope::Eip2930(tx) => tx.tx().to,
        TxEnvelope::Eip4844(tx) => match tx.tx() {
            TxEip4844Variant::TxEip4844(tx) => tx.to(),
            TxEip4844Variant::TxEip4844WithSidecar(tx) => tx.to(),
        },
        _ => {
            unreachable!()
        }
    };

    to.into()
}

pub fn banned_address_call_result(data: Option<&'static RawValue>) -> CallResult {
    Err(ErrorObjectOwned::borrowed(
        ADDRESS_IS_BANNED,
        "The address related to the rpc is banned",
        data,
    ))
}
