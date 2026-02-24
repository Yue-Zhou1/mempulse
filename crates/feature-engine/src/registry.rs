use common::Address;
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::sync::OnceLock;

#[derive(Debug)]
pub struct RegistryError(String);

impl Display for RegistryError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RegistryError {}

#[derive(Debug)]
pub struct ProtocolRegistry {
    by_address: HashMap<Address, &'static str>,
    by_selector: HashMap<[u8; 4], &'static str>,
}

impl ProtocolRegistry {
    pub fn from_json(input: &str) -> Result<Self, RegistryError> {
        let parsed: RegistryConfig = serde_json::from_str(input)
            .map_err(|err| RegistryError(format!("invalid protocol registry json: {err}")))?;

        let mut by_address = HashMap::new();
        let mut by_selector = HashMap::new();

        for protocol in parsed.protocols {
            let name = leak_static(protocol.name);
            for address in protocol.addresses {
                let address = parse_hex_array::<20>(&address)
                    .map_err(|err| RegistryError(format!("invalid protocol address: {err}")))?;
                by_address.insert(address, name);
            }
        }

        for entry in parsed.selector_protocols {
            let selector = parse_hex_array::<4>(&entry.selector)
                .map_err(|err| RegistryError(format!("invalid selector: {err}")))?;
            by_selector.insert(selector, leak_static(entry.protocol));
        }

        Ok(Self {
            by_address,
            by_selector,
        })
    }

    pub fn default_mainnet() -> &'static Self {
        static REGISTRY: OnceLock<ProtocolRegistry> = OnceLock::new();
        REGISTRY.get_or_init(|| {
            ProtocolRegistry::from_json(include_str!(
                "../../../configs/protocol_registry.mainnet.json"
            ))
            .expect("mainnet protocol registry must parse")
        })
    }

    pub fn classify(&self, to: Option<Address>, selector: Option<[u8; 4]>) -> &'static str {
        if let Some(address) = to {
            if let Some(protocol) = self.by_address.get(&address) {
                return protocol;
            }
        }
        if let Some(selector) = selector {
            if let Some(protocol) = self.by_selector.get(&selector) {
                return protocol;
            }
        }
        "unknown"
    }
}

#[derive(Debug, Deserialize)]
struct RegistryConfig {
    protocols: Vec<ProtocolConfigEntry>,
    #[serde(default)]
    selector_protocols: Vec<SelectorProtocolEntry>,
}

#[derive(Debug, Deserialize)]
struct ProtocolConfigEntry {
    name: String,
    #[serde(default)]
    addresses: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct SelectorProtocolEntry {
    selector: String,
    protocol: String,
}

fn leak_static(value: String) -> &'static str {
    Box::leak(value.into_boxed_str())
}

fn parse_hex_array<const N: usize>(value: &str) -> Result<[u8; N], RegistryError> {
    let bytes = parse_hex_bytes(value)?;
    if bytes.len() != N {
        return Err(RegistryError(format!(
            "expected {N} bytes but found {} in {value}",
            bytes.len()
        )));
    }
    let mut out = [0_u8; N];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn parse_hex_bytes(value: &str) -> Result<Vec<u8>, RegistryError> {
    let trimmed = value.trim().strip_prefix("0x").unwrap_or(value.trim());
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    if trimmed.len() % 2 != 0 {
        return Err(RegistryError(format!("odd-length hex string: {value}")));
    }

    let mut out = Vec::with_capacity(trimmed.len() / 2);
    let mut index = 0;
    while index < trimmed.len() {
        let byte = u8::from_str_radix(&trimmed[index..index + 2], 16)
            .map_err(|_| RegistryError(format!("invalid hex string: {value}")))?;
        out.push(byte);
        index += 2;
    }
    Ok(out)
}
