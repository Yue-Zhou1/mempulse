use common::{Address, TxHash};
use event_log::TxDecoded;

#[derive(Clone, Copy, Debug)]
pub struct FeatureInput<'a> {
    pub to: Option<&'a Address>,
    pub calldata: &'a [u8],
    pub tx_type: u8,
    pub chain_id: Option<u64>,
    pub gas_limit: Option<u64>,
    pub value_wei: Option<u128>,
    pub gas_price_wei: Option<u128>,
    pub max_fee_per_gas_wei: Option<u128>,
    pub max_priority_fee_per_gas_wei: Option<u128>,
    pub max_fee_per_blob_gas_wei: Option<u128>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FeatureAnalysis {
    pub protocol: &'static str,
    pub category: &'static str,
    pub mev_score: u16,
    pub urgency_score: u16,
    pub method_selector: Option<[u8; 4]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeaturedTransaction {
    pub hash: TxHash,
    pub analysis: FeatureAnalysis,
    pub tx_type: u8,
    pub to: Option<Address>,
    pub gas_limit: Option<u64>,
    pub calldata_len: usize,
}

const WEI_PER_GWEI: u128 = 1_000_000_000;
const UNISWAP_V2_ROUTER02: Address = [
    0x7a, 0x25, 0x0d, 0x56, 0x30, 0xb4, 0xcf, 0x53, 0x97, 0x39, 0xdf, 0x2c, 0x5d, 0xac, 0xb4,
    0xc6, 0x59, 0xf2, 0x48, 0x8d,
];
const UNISWAP_V3_ROUTER02: Address = [
    0x68, 0xb3, 0x46, 0x58, 0x33, 0xfb, 0x72, 0xa7, 0x0e, 0xcd, 0xf4, 0x85, 0xe0, 0xe4, 0xc7,
    0xbd, 0x86, 0x65, 0xfc, 0x45,
];
const UNISWAP_UNIVERSAL_ROUTER: Address = [
    0xef, 0x1c, 0x6e, 0x67, 0x70, 0x3c, 0x7b, 0xd7, 0x10, 0x7e, 0xed, 0x83, 0x03, 0xfb, 0xe6,
    0xec, 0x25, 0x54, 0xbf, 0x6b,
];
const ONE_INCH_ROUTER_V5: Address = [
    0x11, 0x11, 0x11, 0x12, 0x54, 0xee, 0xb2, 0x54, 0x77, 0xb6, 0x8f, 0xb8, 0x5e, 0xd9, 0x29,
    0xf7, 0x3a, 0x96, 0x05, 0x82,
];

pub fn analyze_transaction(input: FeatureInput<'_>) -> FeatureAnalysis {
    let method_selector = selector(input.calldata);
    let protocol = classify_protocol(input.to, method_selector);
    let category = classify_category(method_selector);
    let urgency_score = compute_urgency_score(input);
    let mut mev_score = base_mev_score(protocol, category);

    if input.calldata.len() > 132 {
        mev_score = mev_score.saturating_add(5);
    }
    if input.value_wei.unwrap_or_default() > 0 {
        mev_score = mev_score.saturating_add(3);
    }
    if input.gas_limit.unwrap_or_default() >= 200_000 {
        mev_score = mev_score.saturating_add(4);
    }
    if input.max_fee_per_blob_gas_wei.is_some() {
        mev_score = mev_score.saturating_add(2);
    }
    mev_score = mev_score.saturating_add(urgency_score / 2);
    if input.chain_id != Some(1) {
        mev_score = mev_score.saturating_sub(15);
    }
    mev_score = mev_score.min(100);

    FeatureAnalysis {
        protocol,
        category,
        mev_score,
        urgency_score,
        method_selector,
    }
}

pub fn analyze_decoded_transaction(tx: &TxDecoded, calldata: &[u8]) -> FeaturedTransaction {
    let analysis = analyze_transaction(FeatureInput {
        to: tx.to.as_ref(),
        calldata,
        tx_type: tx.tx_type,
        chain_id: tx.chain_id,
        gas_limit: tx.gas_limit,
        value_wei: tx.value_wei,
        gas_price_wei: tx.gas_price_wei,
        max_fee_per_gas_wei: tx.max_fee_per_gas_wei,
        max_priority_fee_per_gas_wei: tx.max_priority_fee_per_gas_wei,
        max_fee_per_blob_gas_wei: tx.max_fee_per_blob_gas_wei,
    });

    FeaturedTransaction {
        hash: tx.hash,
        analysis,
        tx_type: tx.tx_type,
        to: tx.to,
        gas_limit: tx.gas_limit,
        calldata_len: calldata.len(),
    }
}

#[inline]
fn selector(calldata: &[u8]) -> Option<[u8; 4]> {
    if calldata.len() < 4 {
        return None;
    }
    Some([calldata[0], calldata[1], calldata[2], calldata[3]])
}

#[inline]
fn classify_protocol(to: Option<&Address>, method_selector: Option<[u8; 4]>) -> &'static str {
    if let Some(address) = to {
        if address == &UNISWAP_V2_ROUTER02 {
            return "uniswap-v2";
        }
        if address == &UNISWAP_V3_ROUTER02 {
            return "uniswap-v3";
        }
        if address == &UNISWAP_UNIVERSAL_ROUTER {
            return "uniswap-universal";
        }
        if address == &ONE_INCH_ROUTER_V5 {
            return "1inch";
        }
    }

    match method_selector {
        Some([0xa9, 0x05, 0x9c, 0xbb]) => "erc20",
        Some([0x23, 0xb8, 0x72, 0xdd]) => "erc20",
        Some([0x09, 0x5e, 0xa7, 0xb3]) => "erc20",
        _ => "unknown",
    }
}

#[inline]
fn classify_category(method_selector: Option<[u8; 4]>) -> &'static str {
    match method_selector {
        // Uniswap V2 swaps.
        Some([0x38, 0xed, 0x17, 0x39])
        | Some([0x88, 0x03, 0xdb, 0xee])
        | Some([0x7f, 0xf3, 0x6a, 0xb5])
        | Some([0x18, 0xcb, 0xaf, 0xe5])
        | Some([0x4a, 0x25, 0xd9, 0x4a])
        | Some([0xfb, 0x3b, 0xdb, 0x41]) =>
            "swap",
        // Uniswap V3 swaps.
        Some([0x04, 0xe4, 0x5a, 0xaf])
        | Some([0xb8, 0x58, 0x18, 0x3f])
        | Some([0x50, 0x23, 0xb4, 0xdf])
        | Some([0x09, 0xb8, 0x13, 0x46]) =>
            "swap",
        // Common ERC20 operations.
        Some([0xa9, 0x05, 0x9c, 0xbb]) | Some([0x23, 0xb8, 0x72, 0xdd]) => "transfer",
        Some([0x09, 0x5e, 0xa7, 0xb3]) => "approval",
        _ => "pending",
    }
}

#[inline]
fn compute_urgency_score(input: FeatureInput<'_>) -> u16 {
    let mut urgency = 0_u16;
    if let Some(priority) = input.max_priority_fee_per_gas_wei {
        let gwei = (priority / WEI_PER_GWEI) as u16;
        urgency = urgency.saturating_add(gwei.min(60));
    } else if let Some(gas_price) = input.gas_price_wei {
        let gwei = (gas_price / WEI_PER_GWEI) as u16;
        urgency = urgency.saturating_add((gwei / 2).min(40));
    }

    if input.tx_type == 2 {
        urgency = urgency.saturating_add(6);
    }
    if input.max_fee_per_gas_wei.unwrap_or_default() > 0 {
        urgency = urgency.saturating_add(4);
    }
    urgency.min(100)
}

#[inline]
fn base_mev_score(protocol: &'static str, category: &'static str) -> u16 {
    let mut score: u16 = match category {
        "swap" => 55,
        "approval" => 16,
        "transfer" => 10,
        _ => 8,
    };
    if matches!(
        protocol,
        "uniswap-v2" | "uniswap-v3" | "uniswap-universal" | "1inch"
    ) {
        score = score.saturating_add(12);
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(byte: u8) -> Address {
        [byte; 20]
    }

    #[test]
    fn classifies_uniswap_v2_swap_and_scores_high() {
        let router = [
            0x7a, 0x25, 0x0d, 0x56, 0x30, 0xb4, 0xcf, 0x53, 0x97, 0x39, 0xdf, 0x2c, 0x5d, 0xac,
            0xb4, 0xc6, 0x59, 0xf2, 0x48, 0x8d,
        ];
        let calldata = [0x38, 0xed, 0x17, 0x39, 0, 0, 0, 0];
        let analysis = analyze_transaction(FeatureInput {
            to: Some(&router),
            calldata: &calldata,
            tx_type: 2,
            chain_id: Some(1),
            gas_limit: Some(250_000),
            value_wei: Some(1_000_000_000_000_000),
            gas_price_wei: Some(40_000_000_000),
            max_fee_per_gas_wei: Some(65_000_000_000),
            max_priority_fee_per_gas_wei: Some(4_000_000_000),
            max_fee_per_blob_gas_wei: None,
        });

        assert_eq!(analysis.protocol, "uniswap-v2");
        assert_eq!(analysis.category, "swap");
        assert_eq!(analysis.method_selector, Some([0x38, 0xed, 0x17, 0x39]));
        assert!(analysis.mev_score >= 60);
        assert!(analysis.urgency_score > 0);
    }

    #[test]
    fn classifies_erc20_transfer_with_low_mev_score() {
        let token = address(0xab);
        let calldata = [0xa9, 0x05, 0x9c, 0xbb, 0, 0, 0, 0];
        let analysis = analyze_transaction(FeatureInput {
            to: Some(&token),
            calldata: &calldata,
            tx_type: 2,
            chain_id: Some(1),
            gas_limit: Some(50_000),
            value_wei: Some(0),
            gas_price_wei: Some(20_000_000_000),
            max_fee_per_gas_wei: Some(25_000_000_000),
            max_priority_fee_per_gas_wei: Some(1_000_000_000),
            max_fee_per_blob_gas_wei: None,
        });

        assert_eq!(analysis.protocol, "erc20");
        assert_eq!(analysis.category, "transfer");
        assert_eq!(analysis.method_selector, Some([0xa9, 0x05, 0x9c, 0xbb]));
        assert!(analysis.mev_score <= 30);
    }
}
