use alloy_primitives::Address;
use async_trait::async_trait;
use serde::{Deserialize, Deserializer};
use std::fmt::Display;
use crate::utils::ToAddress;

use super::{Extension, ExtensionRegistry};

// Read rpc.
pub const ETH_CALL: &'static str = "eth_call";

// Write rpc.
pub const SEND_RAW_TX: &'static str = "eth_sendRawTransaction";
pub const SEND_TX: &'static str = "eth_sendTransaction";

pub struct Whitelist {
    pub config: WhitelistConfig,
}

/// The address whitelist for `eth_call/eth_sendRawTransaction` rpc.
#[derive(Deserialize, Debug, Clone)]
pub struct WhitelistConfig {
    #[serde(default)]
    pub eth_call_whitelist: Vec<WhiteAddress>,
    #[serde(default)]
    pub raw_tx_whitelist: Vec<WhiteAddress>,
    #[serde(default)]
    pub tx_whitelist: Vec<WhiteAddress>,
}

impl WhitelistConfig {
    /// Normalize the config name cases.
    pub fn normalize(&mut self) -> anyhow::Result<()> {
        for addr in self.eth_call_whitelist.iter_mut() {
            addr.normalize()?;
        }

        for addr in self.raw_tx_whitelist.iter_mut() {
            addr.normalize()?;
        }

        for addr in self.tx_whitelist.iter_mut() {
            addr.normalize()?;
        }

        Ok(())
    }
}

/// When an address is None, it means it will satisfy any address.
#[derive(Deserialize, Debug, Clone)]
pub struct WhiteAddress {
    /// Should check the address if Some.
    pub from: Option<Address>,
    /// Should check the address if Some.
    pub to: Option<ToAddress>,
}

impl Whitelist {
    pub fn new(config: WhitelistConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Extension for Whitelist {
    type Config = WhitelistConfig;

    async fn from_config(config: &Self::Config, _registry: &ExtensionRegistry) -> Result<Self, anyhow::Error> {
        let mut config = config.clone();
        config.normalize()?;
        Ok(Self::new(config))
    }
}
