use std::fmt::Display;
use alloy_primitives::{Address, TxKind};
use serde::{Deserialize, Deserializer};
use crate::extensions::whitelist::WhiteAddress;

/// The normalized `to` address:
/// - Create: this call is contract deploy.
/// - Call: this call is contract call.
///
/// Note: this type is similar to [`TxKind`] but different in serde parts.
#[derive(Deserialize, Debug, Clone, Eq, PartialEq)]
#[serde(untagged)]
pub enum ToAddress {
    #[serde(deserialize_with = "deserialize_create")]
    Create,
    Call(Address),
}

/// Helper function to deserialize boxed blobs
fn deserialize_create<'de, D>(deserializer: D) -> Result<(), D::Error>
    where
        D: Deserializer<'de>,
{
    let s = <String>::deserialize(deserializer)?;
    if &s == "create" || &s == "Create" {
        Ok(())
    } else {
        Err(serde::de::Error::custom("invalid `to` address"))
    }
}


impl Display for crate::extensions::whitelist::ToAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Create => write!(f, "create"),
            Self::Call(s) => write!(f, "{}", s),
        }
    }
}

impl From<TxKind> for crate::extensions::whitelist::ToAddress {
    fn from(tx: TxKind) -> Self {
        Self::from(&tx)
    }
}

impl From<&TxKind> for crate::extensions::whitelist::ToAddress {
    fn from(tx: &TxKind) -> Self {
        match tx {
            TxKind::Call(addr) => Self::Call(*addr),
            TxKind::Create => Self::Create,
        }
    }
}

impl From<&Address> for crate::extensions::whitelist::ToAddress {
    fn from(to: &Address) -> Self {
        Self::Call(*to)
    }
}

impl From<Address> for crate::extensions::whitelist::ToAddress {
    fn from(to: Address) -> Self {
        Self::from(&to)
    }
}

impl WhiteAddress {
    /// Normalize the address.
    pub fn normalize(&mut self) -> anyhow::Result<()> {
        println!("white address: {:?}", self);
        // if let Some(addr) = self.from.as_mut() {
        //     let normalized = addr.to_lowercase();
        //     if normalized.len() != 42 {
        //         bail!("Illegal `from` address: {}", addr)
        //     }
        //     *addr = normalized
        // }
        // if let Some(addr) = self.to.as_mut() {
        //     let normalized = addr.to_lowercase();
        //     if normalized.len() != 42 || normalized != "create" {
        //         bail!("Illegal `to` address: {}", addr)
        //     }
        //     *addr = normalized;
        // }

        Ok(())
    }

    /// Check if this is a white address.
    pub fn satisfy(&self, from: &Address, to: &crate::extensions::whitelist::ToAddress) -> bool {
        self.satisfy_from_address(from) && self.satisfy_to_address(to)
    }

    pub fn satisfy_from_address(&self, from: &Address) -> bool {
        self.from == None || self.from.as_ref() == Some(from)
    }

    pub fn satisfy_to_address(&self, to: &crate::extensions::whitelist::ToAddress) -> bool {
        self.to == None || self.to.as_ref() == Some(to)
    }
}