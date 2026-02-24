use ahash::RandomState;
use common::{Address, BlockHash, TxHash};
use event_log::{EventEnvelope, EventPayload, TxDecoded, TxDropped, TxReorged};
use hashbrown::{HashMap, HashSet};

type FastMap<K, V> = HashMap<K, V, RandomState>;
type FastSet<T> = HashSet<T, RandomState>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TxLifecycleStatus {
    Pending,
    Replaced {
        by: TxHash,
    },
    Dropped {
        reason: String,
    },
    ConfirmedProvisional {
        block_number: u64,
        block_hash: BlockHash,
    },
    ConfirmedFinal {
        block_number: u64,
        block_hash: BlockHash,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StateTransition {
    Pending {
        hash: TxHash,
    },
    Replaced {
        old_hash: TxHash,
        new_hash: TxHash,
    },
    Dropped {
        hash: TxHash,
        reason: String,
    },
    ConfirmedProvisional {
        hash: TxHash,
        block_hash: BlockHash,
    },
    ConfirmedFinal {
        hash: TxHash,
        block_hash: BlockHash,
    },
    ReorgReopened {
        hash: TxHash,
        old_block_hash: BlockHash,
        new_block_hash: BlockHash,
    },
}

#[derive(Clone, Debug)]
struct TxEntry {
    sender: Option<Address>,
    nonce: Option<u64>,
    status: TxLifecycleStatus,
}

#[derive(Clone, Debug, Default)]
pub struct MempoolState {
    txs: FastMap<TxHash, TxEntry>,
    sender_nonce_index: FastMap<(Address, u64), TxHash>,
    block_confirmations: FastMap<BlockHash, FastSet<TxHash>>,
}

impl MempoolState {
    pub fn from_pending_hashes(pending_hashes: &[TxHash]) -> Self {
        let mut state = Self::default();
        for hash in pending_hashes {
            state.txs.insert(
                *hash,
                TxEntry {
                    sender: None,
                    nonce: None,
                    status: TxLifecycleStatus::Pending,
                },
            );
        }
        state
    }

    pub fn apply_event(&mut self, event: &EventEnvelope) -> Vec<StateTransition> {
        match &event.payload {
            EventPayload::TxDecoded(decoded) => self.apply_decoded(decoded),
            EventPayload::TxDropped(dropped) => self.apply_dropped(dropped),
            EventPayload::TxConfirmedProvisional(confirmed) => {
                let entry = self.txs.entry(confirmed.hash).or_insert(TxEntry {
                    sender: None,
                    nonce: None,
                    status: TxLifecycleStatus::Pending,
                });
                entry.status = TxLifecycleStatus::ConfirmedProvisional {
                    block_number: confirmed.block_number,
                    block_hash: confirmed.block_hash,
                };
                self.block_confirmations
                    .entry(confirmed.block_hash)
                    .or_default()
                    .insert(confirmed.hash);
                vec![StateTransition::ConfirmedProvisional {
                    hash: confirmed.hash,
                    block_hash: confirmed.block_hash,
                }]
            }
            EventPayload::TxConfirmedFinal(confirmed) => {
                let entry = self.txs.entry(confirmed.hash).or_insert(TxEntry {
                    sender: None,
                    nonce: None,
                    status: TxLifecycleStatus::Pending,
                });
                entry.status = TxLifecycleStatus::ConfirmedFinal {
                    block_number: confirmed.block_number,
                    block_hash: confirmed.block_hash,
                };
                self.block_confirmations
                    .entry(confirmed.block_hash)
                    .or_default()
                    .insert(confirmed.hash);
                vec![StateTransition::ConfirmedFinal {
                    hash: confirmed.hash,
                    block_hash: confirmed.block_hash,
                }]
            }
            EventPayload::TxReorged(reorged) => self.apply_reorg(reorged),
            EventPayload::TxReplaced(replaced) => {
                if let Some(entry) = self.txs.get_mut(&replaced.hash) {
                    entry.status = TxLifecycleStatus::Replaced {
                        by: replaced.replaced_by,
                    };
                }
                vec![StateTransition::Replaced {
                    old_hash: replaced.hash,
                    new_hash: replaced.replaced_by,
                }]
            }
            EventPayload::TxSeen(_)
            | EventPayload::TxFetched(_)
            | EventPayload::OppDetected(_)
            | EventPayload::SimCompleted(_)
            | EventPayload::BundleSubmitted(_) => Vec::new(),
        }
    }

    pub fn lifecycle(&self, hash: &TxHash) -> Option<&TxLifecycleStatus> {
        self.txs.get(hash).map(|entry| &entry.status)
    }

    pub fn pending_hashes(&self) -> Vec<TxHash> {
        self.txs
            .iter()
            .filter_map(|(hash, entry)| match entry.status {
                TxLifecycleStatus::Pending => Some(*hash),
                _ => None,
            })
            .collect()
    }

    fn apply_decoded(&mut self, decoded: &TxDecoded) -> Vec<StateTransition> {
        let mut transitions = Vec::new();
        if let Some(old_hash) = self
            .sender_nonce_index
            .insert((decoded.sender, decoded.nonce), decoded.hash)
        {
            if old_hash != decoded.hash {
                if let Some(old_entry) = self.txs.get_mut(&old_hash) {
                    old_entry.status = TxLifecycleStatus::Replaced { by: decoded.hash };
                }
                transitions.push(StateTransition::Replaced {
                    old_hash,
                    new_hash: decoded.hash,
                });
            }
        }

        let entry = self.txs.entry(decoded.hash).or_insert(TxEntry {
            sender: Some(decoded.sender),
            nonce: Some(decoded.nonce),
            status: TxLifecycleStatus::Pending,
        });
        entry.sender = Some(decoded.sender);
        entry.nonce = Some(decoded.nonce);
        entry.status = TxLifecycleStatus::Pending;
        transitions.push(StateTransition::Pending { hash: decoded.hash });

        transitions
    }

    fn apply_dropped(&mut self, dropped: &TxDropped) -> Vec<StateTransition> {
        let mut transitions = Vec::new();
        if let Some(entry) = self.txs.get_mut(&dropped.hash) {
            if let (Some(sender), Some(nonce)) = (entry.sender, entry.nonce) {
                if self
                    .sender_nonce_index
                    .get(&(sender, nonce))
                    .is_some_and(|hash| *hash == dropped.hash)
                {
                    self.sender_nonce_index.remove(&(sender, nonce));
                }
            }

            entry.status = TxLifecycleStatus::Dropped {
                reason: dropped.reason.clone(),
            };
            transitions.push(StateTransition::Dropped {
                hash: dropped.hash,
                reason: dropped.reason.clone(),
            });
        }
        transitions
    }

    fn apply_reorg(&mut self, reorged: &TxReorged) -> Vec<StateTransition> {
        let mut transitions = Vec::new();
        if let Some(entry) = self.txs.get_mut(&reorged.hash) {
            let should_reopen = matches!(
                entry.status,
                TxLifecycleStatus::ConfirmedProvisional { block_hash, .. }
                    | TxLifecycleStatus::ConfirmedFinal { block_hash, .. }
                    if block_hash == reorged.old_block_hash
            );

            if should_reopen {
                entry.status = TxLifecycleStatus::Pending;
                if let Some(set) = self.block_confirmations.get_mut(&reorged.old_block_hash) {
                    set.remove(&reorged.hash);
                }
                transitions.push(StateTransition::ReorgReopened {
                    hash: reorged.hash,
                    old_block_hash: reorged.old_block_hash,
                    new_block_hash: reorged.new_block_hash,
                });
            }
        }
        transitions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::SourceId;
    use event_log::{TxConfirmed, TxReplaced, TxSeen};

    fn hash(value: u8) -> TxHash {
        [value; 32]
    }

    fn address(value: u8) -> Address {
        [value; 20]
    }

    fn block(value: u8) -> BlockHash {
        [value; 32]
    }

    fn envelope(seq_id: u64, payload: EventPayload) -> EventEnvelope {
        EventEnvelope {
            seq_id,
            ingest_ts_unix_ms: 1_700_000_000_000 + seq_id as i64,
            ingest_ts_mono_ns: seq_id * 10,
            source_id: SourceId::new("test-source"),
            payload,
        }
    }

    fn decoded(hash_value: u8, sender_value: u8, nonce: u64) -> EventPayload {
        EventPayload::TxDecoded(TxDecoded {
            hash: hash(hash_value),
            tx_type: 2,
            sender: address(sender_value),
            nonce,
            chain_id: Some(1),
            to: None,
            value_wei: None,
            gas_limit: None,
            gas_price_wei: None,
            max_fee_per_gas_wei: None,
            max_priority_fee_per_gas_wei: None,
            max_fee_per_blob_gas_wei: None,
            calldata_len: None,
        })
    }

    #[test]
    fn detects_replacement_by_sender_nonce() {
        let mut state = MempoolState::default();

        state.apply_event(&envelope(1, decoded(1, 10, 7)));
        let transitions = state.apply_event(&envelope(2, decoded(2, 10, 7)));

        assert!(transitions.contains(&StateTransition::Replaced {
            old_hash: hash(1),
            new_hash: hash(2),
        }));
        assert_eq!(
            state.lifecycle(&hash(1)),
            Some(&TxLifecycleStatus::Replaced { by: hash(2) })
        );
        assert_eq!(state.lifecycle(&hash(2)), Some(&TxLifecycleStatus::Pending));
    }

    #[test]
    fn reorg_rolls_back_confirmed_to_pending() {
        let mut state = MempoolState::default();

        state.apply_event(&envelope(1, decoded(3, 9, 1)));
        state.apply_event(&envelope(
            2,
            EventPayload::TxConfirmedProvisional(TxConfirmed {
                hash: hash(3),
                block_number: 1_000,
                block_hash: block(5),
            }),
        ));
        let transitions = state.apply_event(&envelope(
            3,
            EventPayload::TxReorged(TxReorged {
                hash: hash(3),
                old_block_hash: block(5),
                new_block_hash: block(6),
            }),
        ));

        assert_eq!(state.lifecycle(&hash(3)), Some(&TxLifecycleStatus::Pending));
        assert_eq!(
            transitions,
            vec![StateTransition::ReorgReopened {
                hash: hash(3),
                old_block_hash: block(5),
                new_block_hash: block(6),
            }]
        );
    }

    #[test]
    fn dropped_tx_removes_sender_nonce_index() {
        let mut state = MempoolState::default();

        state.apply_event(&envelope(1, decoded(7, 2, 11)));
        state.apply_event(&envelope(
            2,
            EventPayload::TxDropped(TxDropped {
                hash: hash(7),
                reason: "evicted".to_owned(),
            }),
        ));
        let transitions = state.apply_event(&envelope(3, decoded(8, 2, 11)));

        assert_eq!(state.lifecycle(&hash(8)), Some(&TxLifecycleStatus::Pending));
        assert!(!transitions.contains(&StateTransition::Replaced {
            old_hash: hash(7),
            new_hash: hash(8),
        }));
    }

    #[test]
    fn ignores_unrelated_payload_without_panicking() {
        let mut state = MempoolState::default();
        let transitions = state.apply_event(&envelope(
            1,
            EventPayload::TxSeen(TxSeen {
                hash: hash(1),
                peer_id: "peer-a".to_owned(),
                seen_at_unix_ms: 1_700_000_000_000,
                seen_at_mono_ns: 10,
            }),
        ));
        assert!(transitions.is_empty());
    }

    #[test]
    fn explicit_replaced_payload_sets_lifecycle() {
        let mut state = MempoolState::default();
        state.apply_event(&envelope(1, decoded(9, 4, 1)));
        state.apply_event(&envelope(
            2,
            EventPayload::TxReplaced(TxReplaced {
                hash: hash(9),
                replaced_by: hash(10),
            }),
        ));
        assert_eq!(
            state.lifecycle(&hash(9)),
            Some(&TxLifecycleStatus::Replaced { by: hash(10) })
        );
    }
}
