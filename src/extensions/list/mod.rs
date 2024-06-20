use crate::utils::AddressRule;
use async_trait::async_trait;
use serde::Deserialize;

use super::{Extension, ExtensionRegistry};

// Read rpc.
pub const ETH_CALL: &str = "eth_call";

// Write rpc.
pub const SEND_RAW_TX: &str = "eth_sendRawTransaction";
pub const SEND_TX: &str = "eth_sendTransaction";

/// The address whitelist for `eth_call/eth_sendRawTransaction` rpc.
#[derive(Deserialize, Debug, Clone)]
pub struct WhitelistConfig {
    #[serde(default, alias = "eth_call")]
    pub eth_call_whitelist: Vec<AddressRule>,
    #[serde(default, alias = "tx")]
    pub tx_whitelist: Vec<AddressRule>,
}

/// The address whitelist for `eth_call/eth_sendRawTransaction` rpc.
#[derive(Deserialize, Debug, Clone)]
pub struct BlacklistConfig {
    #[serde(default, alias = "eth_call")]
    pub eth_call_whitelist: Vec<AddressRule>,
    #[serde(default, alias = "tx")]
    pub tx_whitelist: Vec<AddressRule>,
}

pub struct Whitelist {
    pub config: WhitelistConfig,
}

pub struct BlackList {
    pub config: BlacklistConfig,
}


#[async_trait]
impl Extension for Whitelist {
    type Config = WhitelistConfig;

    async fn from_config(config: &Self::Config, _registry: &ExtensionRegistry) -> Result<Self, anyhow::Error> {
        Ok(Self {
            config: config.clone(),
        })
    }
}

#[async_trait]
impl Extension for BlackList {
    type Config = BlacklistConfig;

    async fn from_config(config: &Self::Config, _registry: &ExtensionRegistry) -> Result<Self, anyhow::Error> {
        Ok(Self {
            config: config.clone(),
        })
    }
}
