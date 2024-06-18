use async_trait::async_trait;
use serde::Deserialize;

use super::{Extension, ExtensionRegistry};

pub const SEND_RAW: &'static str = "eth_sendRawTransaction";
pub const ETH_CALL: &'static str = "eth_call";

pub struct Whitelist {
    pub config: WhitelistConfig,
}

/// The address whitelist for `eth_call/eth_sendRawTransaction` rpc.
#[derive(Deserialize, Debug, Clone)]
pub struct WhitelistConfig {
    pub eth_call_whitelist: Vec<WhiteAddress>,
    pub raw_tx_whitelist: Vec<WhiteAddress>,
}

impl WhitelistConfig {
    /// Normalize the config name cases.
    pub fn normalize(&mut self) {
        for addr in self.eth_call_whitelist.iter_mut() {
            addr.normalize();
        }

        for addr in self.raw_tx_whitelist.iter_mut() {
            addr.normalize();
        }
    }
}

/// When an address is None, it means it will satisfy any address.
#[derive(Deserialize, Debug, Clone)]
pub struct WhiteAddress {
    /// Should check the address if Some.
    pub from: Option<String>,
    /// Should check the address if Some.
    pub to: Option<String>,
}

impl WhiteAddress {
    /// Normalize the address.
    pub fn normalize(&mut self) {
        if let Some(addr) = self.from.as_mut() {
            *addr = addr.to_lowercase();
        }
        if let Some(addr) = self.to.as_mut() {
            *addr = addr.to_lowercase();
        }
    }

    pub fn satisfy_from_address(&self, from: impl AsRef<str>) -> bool {
        let from = from.as_ref();

        self.from.as_deref() == None || self.from.as_deref() == Some(from)
    }

    pub fn satisfy_to_address(&self, to: impl AsRef<str>) -> bool {
        let to = to.as_ref();

        self.to.as_deref() == None || self.to.as_deref() == Some(to)
    }

    /// Check if this is a white address.
    pub fn satisfy(&self, from: impl AsRef<str>, to: impl AsRef<str>) -> bool {
        self.satisfy_from_address(from) && self.satisfy_to_address(to)
    }
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
        config.normalize();
        Ok(Self::new(config))
    }
}