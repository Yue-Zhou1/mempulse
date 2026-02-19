use common::{Address, TxHash};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TxType {
    Legacy,
    Eip2930,
    Eip1559,
    Eip4844,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RawTxInput {
    pub hash: String,
    pub tx_type: TxType,
    pub chain_id: u64,
    pub sender: String,
    pub nonce: u64,
    pub to: Option<String>,
    pub value: u128,
    pub gas_limit: u64,
    pub gas_price: Option<u128>,
    pub max_fee_per_gas: Option<u128>,
    pub max_priority_fee_per_gas: Option<u128>,
    pub max_fee_per_blob_gas: Option<u128>,
    pub calldata: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NormalizedFees {
    pub gas_price: Option<u128>,
    pub max_fee_per_gas: Option<u128>,
    pub max_priority_fee_per_gas: Option<u128>,
    pub max_fee_per_blob_gas: Option<u128>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DecodedTx {
    pub hash: TxHash,
    pub tx_type: TxType,
    pub chain_id: u64,
    pub sender: Address,
    pub nonce: u64,
    pub to: Option<Address>,
    pub value: u128,
    pub gas_limit: u64,
    pub fees: NormalizedFees,
    pub calldata: Vec<u8>,
}

impl DecodedTx {
    pub fn effective_gas_price(&self, base_fee: u128) -> Option<u128> {
        match self.tx_type {
            TxType::Legacy | TxType::Eip2930 => self.fees.gas_price,
            TxType::Eip1559 | TxType::Eip4844 => {
                let max_fee = self.fees.max_fee_per_gas?;
                let priority = self.fees.max_priority_fee_per_gas?;
                Some(max_fee.min(base_fee.saturating_add(priority)))
            }
        }
    }
}

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("invalid hex value for field '{field}'")]
    InvalidHex { field: &'static str },
    #[error("invalid length for field '{field}', expected {expected} bytes")]
    InvalidLength {
        field: &'static str,
        expected: usize,
    },
    #[error("missing required fee field '{field}' for tx type")]
    MissingFeeField { field: &'static str },
    #[error("json decode failed: {0}")]
    JsonDecode(#[from] serde_json::Error),
}

pub fn decode_from_json_str(input: &str) -> Result<DecodedTx, DecodeError> {
    let raw: RawTxInput = serde_json::from_str(input)?;
    decode_from_raw(raw)
}

pub fn decode_from_json_bytes(input: &[u8]) -> Result<DecodedTx, DecodeError> {
    let raw: RawTxInput = serde_json::from_slice(input)?;
    decode_from_raw(raw)
}

pub fn decode_from_raw(input: RawTxInput) -> Result<DecodedTx, DecodeError> {
    let fees = normalize_fees(&input)?;
    let hash = parse_fixed_hex::<32>(&input.hash, "hash")?;
    let sender = parse_fixed_hex::<20>(&input.sender, "sender")?;

    let to = match input.to.as_ref() {
        None => None,
        Some(value) if value.is_empty() => None,
        Some(value) => Some(parse_fixed_hex::<20>(value, "to")?),
    };

    let calldata = match input.calldata {
        None => Vec::new(),
        Some(value) if value.is_empty() => Vec::new(),
        Some(value) => parse_variable_hex(&value, "calldata")?,
    };

    Ok(DecodedTx {
        hash,
        tx_type: input.tx_type,
        chain_id: input.chain_id,
        sender,
        nonce: input.nonce,
        to,
        value: input.value,
        gas_limit: input.gas_limit,
        fees,
        calldata,
    })
}

fn normalize_fees(input: &RawTxInput) -> Result<NormalizedFees, DecodeError> {
    match input.tx_type {
        TxType::Legacy | TxType::Eip2930 => {
            let gas_price = input
                .gas_price
                .ok_or(DecodeError::MissingFeeField { field: "gas_price" })?;
            Ok(NormalizedFees {
                gas_price: Some(gas_price),
                max_fee_per_gas: None,
                max_priority_fee_per_gas: None,
                max_fee_per_blob_gas: None,
            })
        }
        TxType::Eip1559 => {
            let max_fee = input.max_fee_per_gas.ok_or(DecodeError::MissingFeeField {
                field: "max_fee_per_gas",
            })?;
            let priority = input
                .max_priority_fee_per_gas
                .ok_or(DecodeError::MissingFeeField {
                    field: "max_priority_fee_per_gas",
                })?;
            Ok(NormalizedFees {
                gas_price: None,
                max_fee_per_gas: Some(max_fee),
                max_priority_fee_per_gas: Some(priority),
                max_fee_per_blob_gas: None,
            })
        }
        TxType::Eip4844 => {
            let max_fee = input.max_fee_per_gas.ok_or(DecodeError::MissingFeeField {
                field: "max_fee_per_gas",
            })?;
            let priority = input
                .max_priority_fee_per_gas
                .ok_or(DecodeError::MissingFeeField {
                    field: "max_priority_fee_per_gas",
                })?;
            let blob_fee = input
                .max_fee_per_blob_gas
                .ok_or(DecodeError::MissingFeeField {
                    field: "max_fee_per_blob_gas",
                })?;
            Ok(NormalizedFees {
                gas_price: None,
                max_fee_per_gas: Some(max_fee),
                max_priority_fee_per_gas: Some(priority),
                max_fee_per_blob_gas: Some(blob_fee),
            })
        }
    }
}

fn parse_fixed_hex<const N: usize>(
    value: &str,
    field: &'static str,
) -> Result<[u8; N], DecodeError> {
    let bytes = parse_variable_hex(value, field)?;
    if bytes.len() != N {
        return Err(DecodeError::InvalidLength { field, expected: N });
    }
    let mut out = [0_u8; N];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn parse_variable_hex(value: &str, field: &'static str) -> Result<Vec<u8>, DecodeError> {
    let trimmed = value.strip_prefix("0x").unwrap_or(value);
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    if trimmed.len() % 2 != 0 {
        return Err(DecodeError::InvalidHex { field });
    }

    (0..trimmed.len())
        .step_by(2)
        .map(|idx| {
            u8::from_str_radix(&trimmed[idx..idx + 2], 16)
                .map_err(|_| DecodeError::InvalidHex { field })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_raw(tx_type: TxType) -> RawTxInput {
        RawTxInput {
            hash: format!("0x{}", "11".repeat(32)),
            tx_type,
            chain_id: 1,
            sender: format!("0x{}", "22".repeat(20)),
            nonce: 7,
            to: Some(format!("0x{}", "33".repeat(20))),
            value: 12_345,
            gas_limit: 30_000,
            gas_price: Some(55),
            max_fee_per_gas: Some(70),
            max_priority_fee_per_gas: Some(3),
            max_fee_per_blob_gas: Some(5),
            calldata: Some("0xaabbccdd".to_owned()),
        }
    }

    #[test]
    fn decodes_legacy_transaction() {
        let mut input = sample_raw(TxType::Legacy);
        input.max_fee_per_gas = None;
        input.max_priority_fee_per_gas = None;
        input.max_fee_per_blob_gas = None;

        let decoded = decode_from_raw(input).expect("legacy decode");
        assert_eq!(decoded.tx_type, TxType::Legacy);
        assert_eq!(decoded.fees.gas_price, Some(55));
        assert_eq!(decoded.fees.max_fee_per_gas, None);
        assert_eq!(decoded.calldata, vec![0xaa, 0xbb, 0xcc, 0xdd]);
    }

    #[test]
    fn decodes_contract_creation_to_none() {
        let mut input = sample_raw(TxType::Eip1559);
        input.to = None;

        let decoded = decode_from_raw(input).expect("decode contract creation");
        assert_eq!(decoded.to, None);
    }

    #[test]
    fn normalizes_eip1559_and_effective_fee() {
        let mut input = sample_raw(TxType::Eip1559);
        input.gas_price = None;
        input.max_fee_per_blob_gas = None;

        let decoded = decode_from_raw(input).expect("eip1559 decode");
        assert_eq!(decoded.fees.max_fee_per_gas, Some(70));
        assert_eq!(decoded.fees.max_priority_fee_per_gas, Some(3));
        assert_eq!(decoded.effective_gas_price(68), Some(70));
        assert_eq!(decoded.effective_gas_price(10), Some(13));
    }

    #[test]
    fn requires_blob_fee_for_eip4844() {
        let mut input = sample_raw(TxType::Eip4844);
        input.gas_price = None;
        input.max_fee_per_blob_gas = None;

        let err = decode_from_raw(input).expect_err("should fail");
        assert!(matches!(
            err,
            DecodeError::MissingFeeField {
                field: "max_fee_per_blob_gas"
            }
        ));
    }

    #[test]
    fn decodes_from_json_bytes() {
        let mut input = sample_raw(TxType::Eip2930);
        input.max_fee_per_gas = None;
        input.max_priority_fee_per_gas = None;
        input.max_fee_per_blob_gas = None;
        let encoded = serde_json::to_vec(&input).expect("encode json");

        let decoded = decode_from_json_bytes(&encoded).expect("decode json bytes");
        assert_eq!(decoded.tx_type, TxType::Eip2930);
        assert_eq!(decoded.fees.gas_price, Some(55));
    }
}
