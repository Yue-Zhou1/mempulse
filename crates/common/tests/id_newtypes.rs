use common::{CandidateId, StrategyId};

#[test]
fn candidate_and_strategy_ids_roundtrip_through_serde() {
    let candidate = CandidateId::from("cand-1");
    let strategy = StrategyId::from("SandwichCandidate");

    assert_eq!(serde_json::to_string(&candidate).expect("serialize"), "\"cand-1\"");
    assert_eq!(
        serde_json::to_string(&strategy).expect("serialize"),
        "\"SandwichCandidate\""
    );
    assert_eq!(
        serde_json::from_str::<CandidateId>("\"cand-1\"").expect("deserialize"),
        candidate
    );
    assert_eq!(
        serde_json::from_str::<StrategyId>("\"SandwichCandidate\"").expect("deserialize"),
        strategy
    );
}

#[test]
fn candidate_and_strategy_ids_display_the_inner_value() {
    let candidate = CandidateId::from("cand-99");
    let strategy = StrategyId::from("BundleCandidate");

    assert_eq!(candidate.as_str(), "cand-99");
    assert_eq!(strategy.as_str(), "BundleCandidate");
    assert_eq!(candidate.to_string(), "cand-99");
    assert_eq!(strategy.to_string(), "BundleCandidate");
    assert_eq!(&*candidate, "cand-99");
    assert_eq!(&*strategy, "BundleCandidate");
}
