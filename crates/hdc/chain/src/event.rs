//! Event sync handler -- processes chain events to update the local index.
//!
//! Each Solidity event is identified by its topic\[0\] (the keccak256 of the
//! canonical event signature). This module computes those hashes at startup,
//! decodes the ABI-encoded log data, and applies the mutations to the
//! [`OnChainHdcIndex`].

use alloy_primitives::{Address, B256, U256};
use tracing::{debug, warn};

use crate::index::{InsightState, OnChainHdcIndex};

// ─── Event signatures (canonical, no parameter names) ─────────────────────────

/// Canonical event signature strings matching the Solidity ABI.
mod signatures {
    pub(crate) const INSIGHT_PUBLISHED: &str =
        "InsightPublished(bytes32,bytes32,address,bytes,bytes,uint8,uint8)";
    pub(crate) const INSIGHT_CONFIRMED: &str = "InsightConfirmed(bytes32,address,uint64)";
    pub(crate) const INSIGHT_CHALLENGED: &str = "InsightChallenged(bytes32,bytes32,address)";
    pub(crate) const INSIGHT_STATE_CHANGED: &str = "InsightStateChanged(bytes32,uint8,uint8)";
    pub(crate) const INSIGHT_PURGED: &str = "InsightPurged(bytes32,address)";
    pub(crate) const PHEROMONE_DEPOSITED: &str =
        "PheromoneDeposited(bytes32,bytes32,address,uint8,uint64,uint64)";
}

/// Precomputed keccak256 topic hashes for each event signature.
///
/// Computed once at startup via [`LazyLock`] so we avoid any possibility
/// of stale hard-coded hex strings.
pub mod topics {
    use std::sync::LazyLock;

    use alloy_primitives::{B256, keccak256};

    use super::signatures;

    /// `keccak256("InsightPublished(bytes32,bytes32,address,bytes,bytes,uint8,uint8)")`
    pub static INSIGHT_PUBLISHED: LazyLock<B256> =
        LazyLock::new(|| keccak256(signatures::INSIGHT_PUBLISHED));

    /// `keccak256("InsightConfirmed(bytes32,address,uint64)")`
    pub static INSIGHT_CONFIRMED: LazyLock<B256> =
        LazyLock::new(|| keccak256(signatures::INSIGHT_CONFIRMED));

    /// `keccak256("InsightChallenged(bytes32,bytes32,address)")`
    pub static INSIGHT_CHALLENGED: LazyLock<B256> =
        LazyLock::new(|| keccak256(signatures::INSIGHT_CHALLENGED));

    /// `keccak256("InsightStateChanged(bytes32,uint8,uint8)")`
    pub static INSIGHT_STATE_CHANGED: LazyLock<B256> =
        LazyLock::new(|| keccak256(signatures::INSIGHT_STATE_CHANGED));

    /// `keccak256("InsightPurged(bytes32,address)")`
    pub static INSIGHT_PURGED: LazyLock<B256> =
        LazyLock::new(|| keccak256(signatures::INSIGHT_PURGED));

    /// `keccak256("PheromoneDeposited(bytes32,bytes32,address,uint8,uint64,uint64)")`
    pub static PHEROMONE_DEPOSITED: LazyLock<B256> =
        LazyLock::new(|| keccak256(signatures::PHEROMONE_DEPOSITED));
}

// ─── Error type ──────────────────────────────────────────────────────────────

/// Errors that can occur when processing an event log.
#[derive(Debug, thiserror::Error)]
pub enum EventError {
    /// The log has no topic[0].
    #[error("log has no topics")]
    NoTopics,
    /// An indexed topic is missing from the log.
    #[error("missing indexed topic at position {0}")]
    MissingTopic(usize),
    /// ABI data is too short to contain expected fields.
    #[error("log data too short: need {expected} bytes, got {actual}")]
    DataTooShort {
        /// Minimum number of bytes required.
        expected: usize,
        /// Actual number of bytes in the data.
        actual: usize,
    },
    /// A uint8 value does not map to a known enum variant.
    #[error("invalid enum value {value} for {field}")]
    InvalidEnumValue {
        /// Name of the enum field.
        field: &'static str,
        /// The invalid raw value.
        value: u8,
    },
    /// Unknown event topic -- not necessarily an error, just unhandled.
    #[error("unknown event topic: {0}")]
    UnknownTopic(B256),
}

// ─── ABI decoding helpers ────────────────────────────────────────────────────

/// Read a 32-byte slot from ABI-encoded data at the given word index.
fn read_word(data: &[u8], word_index: usize) -> Result<[u8; 32], EventError> {
    let offset = word_index * 32;
    let end = offset + 32;
    if data.len() < end {
        return Err(EventError::DataTooShort { expected: end, actual: data.len() });
    }
    let mut word = [0u8; 32];
    word.copy_from_slice(&data[offset..end]);
    Ok(word)
}

/// Read a uint8 from a 32-byte ABI word at the given index.
fn read_u8(data: &[u8], word_index: usize) -> Result<u8, EventError> {
    let word = read_word(data, word_index)?;
    // Solidity pads uint8 to 32 bytes, value is in the last byte.
    Ok(word[31])
}

/// Read a uint64 from a 32-byte ABI word at the given index.
fn read_u64(data: &[u8], word_index: usize) -> Result<u64, EventError> {
    let word = read_word(data, word_index)?;
    let val = U256::from_be_bytes(word);
    // Safely truncate -- if the value exceeds u64 we still take the low bits.
    Ok(val.as_limbs()[0])
}

/// Read dynamic `bytes` from ABI-encoded data.
/// `word_index` points to the offset pointer word.
fn read_dynamic_bytes(data: &[u8], word_index: usize) -> Result<Vec<u8>, EventError> {
    let offset_word = read_word(data, word_index)?;
    let offset = U256::from_be_bytes(offset_word).try_into().unwrap_or(usize::MAX);
    if offset + 32 > data.len() {
        return Err(EventError::DataTooShort { expected: offset + 32, actual: data.len() });
    }
    let mut len_bytes = [0u8; 32];
    len_bytes.copy_from_slice(&data[offset..offset + 32]);
    let len: usize = U256::from_be_bytes(len_bytes).try_into().unwrap_or(usize::MAX);
    let start = offset + 32;
    let end = start + len;
    if end > data.len() {
        return Err(EventError::DataTooShort { expected: end, actual: data.len() });
    }
    Ok(data[start..end].to_vec())
}

/// Extract `Address` from an indexed topic (right-aligned in 32 bytes).
fn address_from_topic(topic: &B256) -> Address {
    Address::from_slice(&topic[12..32])
}

// ─── State conversion ───────────────────────────────────────────────────────

/// Map a Solidity `uint8` state value to `InsightState`.
///
/// The ordering matches the Solidity enum:
/// 0=Draft, 1=Submitted, 2=Challenged, 3=Voting, 4=Accepted, 5=Rejected, 6=Expired
fn insight_state_from_u8(val: u8) -> Result<InsightState, EventError> {
    match val {
        0 => Ok(InsightState::Draft),
        1 => Ok(InsightState::Submitted),
        2 => Ok(InsightState::Challenged),
        3 => Ok(InsightState::Voting),
        4 => Ok(InsightState::Accepted),
        5 => Ok(InsightState::Rejected),
        6 => Ok(InsightState::Expired),
        _ => Err(EventError::InvalidEnumValue { field: "InsightState", value: val }),
    }
}

// ─── Main entry point ───────────────────────────────────────────────────────

/// Process a log entry from a finalized block and update the on-chain index.
///
/// `topics` contains the indexed event parameters (topic\[0\] is the event
/// selector). `data` contains ABI-encoded non-indexed parameters.
///
/// Returns `Ok(())` on success, or an `EventError` describing what went wrong.
pub fn process_log(
    index: &mut OnChainHdcIndex,
    topics: &[B256],
    data: &[u8],
    _log_address: &Address,
) -> Result<(), EventError> {
    let topic0 = topics.first().ok_or(EventError::NoTopics)?;

    if *topic0 == *topics::INSIGHT_PUBLISHED {
        process_insight_published(index, topics, data)
    } else if *topic0 == *topics::INSIGHT_CONFIRMED {
        process_insight_confirmed(index, topics, data)
    } else if *topic0 == *topics::INSIGHT_CHALLENGED {
        process_insight_challenged(index, topics)
    } else if *topic0 == *topics::INSIGHT_STATE_CHANGED {
        process_insight_state_changed(index, topics, data)
    } else if *topic0 == *topics::INSIGHT_PURGED {
        process_insight_purged(index, topics)
    } else if *topic0 == *topics::PHEROMONE_DEPOSITED {
        process_pheromone_deposited(index, topics, data)
    } else {
        debug!(topic = %topic0, "ignoring unrecognized event");
        Err(EventError::UnknownTopic(*topic0))
    }
}

// ─── Per-event handlers ─────────────────────────────────────────────────────

/// Decode and apply `InsightPublished`.
///
/// Indexed: insightId (topic[1]), vectorHash (topic[2]), author (topic[3])
/// Data: vector (bytes), content (bytes), kind (uint8), tier (uint8)
fn process_insight_published(
    index: &mut OnChainHdcIndex,
    topics: &[B256],
    data: &[u8],
) -> Result<(), EventError> {
    let _insight_id = topics.get(1).ok_or(EventError::MissingTopic(1))?;
    let _vector_hash = topics.get(2).ok_or(EventError::MissingTopic(2))?;
    let author_topic = topics.get(3).ok_or(EventError::MissingTopic(3))?;
    let author = address_from_topic(author_topic);

    // ABI layout for non-indexed params:
    //   word 0: offset to `vector` (bytes)
    //   word 1: offset to `content` (bytes)
    //   word 2: kind (uint8)
    //   word 3: tier (uint8)
    let vector_bytes = read_dynamic_bytes(data, 0)?;
    let _content_bytes = read_dynamic_bytes(data, 1)?;
    let _kind = read_u8(data, 2)?;
    let _tier = read_u8(data, 3)?;

    // Deserialize the HDC vector from the raw bytes.
    if vector_bytes.len() != kora_hdc::BYTES {
        return Err(EventError::DataTooShort {
            expected: kora_hdc::BYTES,
            actual: vector_bytes.len(),
        });
    }
    let mut buf = [0u8; kora_hdc::BYTES];
    buf.copy_from_slice(&vector_bytes);
    let vector = kora_hdc::deserialize(&buf);

    // block_number is not in the event data -- the caller should attach it.
    // For now use 0; the runner will provide the real block number.
    match index.insert_insight(vector, author, 0) {
        Ok(id) => debug!(id = %id, author = %author, "indexed InsightPublished"),
        Err(e) => warn!(author = %author, error = %e, "failed to index InsightPublished"),
    }
    Ok(())
}

/// Decode and apply `InsightConfirmed`.
///
/// Indexed: insightId (topic[1]), confirmer (topic[2])
/// Data: totalConfirmations (uint64)
fn process_insight_confirmed(
    index: &mut OnChainHdcIndex,
    topics: &[B256],
    data: &[u8],
) -> Result<(), EventError> {
    let insight_id = topics.get(1).ok_or(EventError::MissingTopic(1))?;
    let confirmer_topic = topics.get(2).ok_or(EventError::MissingTopic(2))?;
    let confirmer = address_from_topic(confirmer_topic);
    let total_confirmations = read_u64(data, 0)?;

    // A confirmed insight transitions to Accepted.
    if let Err(e) = index.update_state(insight_id, InsightState::Accepted) {
        warn!(id = %insight_id, error = %e, "InsightConfirmed for unknown insight");
    }
    debug!(
        id = %insight_id,
        confirmer = %confirmer,
        total = total_confirmations,
        "processed InsightConfirmed"
    );
    Ok(())
}

/// Decode and apply `InsightChallenged`.
///
/// Indexed: insightId (topic[1]), challengingInsightId (topic[2]), challenger (topic[3])
/// Data: (none)
fn process_insight_challenged(
    index: &mut OnChainHdcIndex,
    topics: &[B256],
) -> Result<(), EventError> {
    let insight_id = topics.get(1).ok_or(EventError::MissingTopic(1))?;
    let _challenging_id = topics.get(2).ok_or(EventError::MissingTopic(2))?;
    let challenger_topic = topics.get(3).ok_or(EventError::MissingTopic(3))?;
    let challenger = address_from_topic(challenger_topic);

    if let Err(e) = index.update_state(insight_id, InsightState::Challenged) {
        warn!(id = %insight_id, error = %e, "InsightChallenged for unknown insight");
    }
    debug!(id = %insight_id, challenger = %challenger, "processed InsightChallenged");
    Ok(())
}

/// Decode and apply `InsightStateChanged`.
///
/// Indexed: insightId (topic[1])
/// Data: oldState (uint8), newState (uint8)
fn process_insight_state_changed(
    index: &mut OnChainHdcIndex,
    topics: &[B256],
    data: &[u8],
) -> Result<(), EventError> {
    let insight_id = topics.get(1).ok_or(EventError::MissingTopic(1))?;
    let _old_state_val = read_u8(data, 0)?;
    let new_state_val = read_u8(data, 1)?;
    let new_state = insight_state_from_u8(new_state_val)?;

    if let Err(e) = index.update_state(insight_id, new_state) {
        warn!(id = %insight_id, error = %e, "InsightStateChanged for unknown insight");
    }
    debug!(id = %insight_id, new_state = ?new_state, "processed InsightStateChanged");
    Ok(())
}

/// Decode and apply `InsightPurged`.
///
/// Indexed: insightId (topic[1]), purger (topic[2])
/// Data: (none)
///
/// Marks the insight as `Expired` so it is excluded from search results.
/// A full removal would require `OnChainHdcIndex::remove()` which is not
/// yet implemented; marking as Expired achieves the same functional effect
/// since the search method only returns `Accepted` insights.
fn process_insight_purged(index: &mut OnChainHdcIndex, topics: &[B256]) -> Result<(), EventError> {
    let insight_id = topics.get(1).ok_or(EventError::MissingTopic(1))?;
    let purger_topic = topics.get(2).ok_or(EventError::MissingTopic(2))?;
    let purger = address_from_topic(purger_topic);

    if let Err(e) = index.update_state(insight_id, InsightState::Expired) {
        warn!(id = %insight_id, error = %e, "InsightPurged for unknown insight");
    }
    debug!(id = %insight_id, purger = %purger, "processed InsightPurged");
    Ok(())
}

/// Decode and apply `PheromoneDeposited`.
///
/// Indexed: pheromoneId (topic[1]), locationHash (topic[2]), depositor (topic[3])
/// Data: pType (uint8), intensity (uint64), blockNumber (uint64)
fn process_pheromone_deposited(
    index: &mut OnChainHdcIndex,
    topics: &[B256],
    data: &[u8],
) -> Result<(), EventError> {
    let pheromone_id = topics.get(1).ok_or(EventError::MissingTopic(1))?;
    let location_hash = topics.get(2).ok_or(EventError::MissingTopic(2))?;
    let depositor_topic = topics.get(3).ok_or(EventError::MissingTopic(3))?;
    let _depositor = address_from_topic(depositor_topic);

    let _p_type = read_u8(data, 0)?;
    let intensity = read_u64(data, 1)?;
    let _block_number = read_u64(data, 2)?;

    index.record_pheromone(*pheromone_id, *location_hash, intensity);
    debug!(
        pheromone = %pheromone_id,
        location = %location_hash,
        intensity,
        "processed PheromoneDeposited"
    );
    Ok(())
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use alloy_primitives::keccak256;

    use super::*;

    /// Verify that topic hashes are non-zero and deterministic.
    #[test]
    fn topic_hashes_are_nonzero_and_deterministic() {
        assert_ne!(*topics::INSIGHT_PUBLISHED, B256::ZERO);
        assert_ne!(*topics::INSIGHT_CONFIRMED, B256::ZERO);
        assert_ne!(*topics::INSIGHT_CHALLENGED, B256::ZERO);
        assert_ne!(*topics::INSIGHT_STATE_CHANGED, B256::ZERO);
        assert_ne!(*topics::INSIGHT_PURGED, B256::ZERO);
        assert_ne!(*topics::PHEROMONE_DEPOSITED, B256::ZERO);

        // Verify they match the keccak256 of the canonical signature.
        assert_eq!(*topics::INSIGHT_PUBLISHED, keccak256(signatures::INSIGHT_PUBLISHED));
        assert_eq!(*topics::PHEROMONE_DEPOSITED, keccak256(signatures::PHEROMONE_DEPOSITED));
    }

    /// All six topics must be distinct.
    #[test]
    fn topic_hashes_are_all_distinct() {
        let all = [
            *topics::INSIGHT_PUBLISHED,
            *topics::INSIGHT_CONFIRMED,
            *topics::INSIGHT_CHALLENGED,
            *topics::INSIGHT_STATE_CHANGED,
            *topics::INSIGHT_PURGED,
            *topics::PHEROMONE_DEPOSITED,
        ];
        for i in 0..all.len() {
            for j in (i + 1)..all.len() {
                assert_ne!(all[i], all[j], "topics {i} and {j} collide");
            }
        }
    }

    /// Empty topics slice yields `NoTopics` error.
    #[test]
    fn process_log_no_topics_errors() {
        let mut index = OnChainHdcIndex::new();
        let result = process_log(&mut index, &[], &[], &Address::ZERO);
        assert!(matches!(result, Err(EventError::NoTopics)));
    }

    /// Unknown topic yields `UnknownTopic` error.
    #[test]
    fn process_log_unknown_topic() {
        let mut index = OnChainHdcIndex::new();
        let fake = B256::from([0xAA; 32]);
        let result = process_log(&mut index, &[fake], &[], &Address::ZERO);
        assert!(matches!(result, Err(EventError::UnknownTopic(_))));
    }

    /// Missing indexed topic yields `MissingTopic`.
    #[test]
    fn insight_confirmed_missing_topics() {
        let mut index = OnChainHdcIndex::new();
        // Only topic[0], missing topic[1] and topic[2].
        let topics = vec![*topics::INSIGHT_CONFIRMED];
        let result = process_log(&mut index, &topics, &[], &Address::ZERO);
        assert!(matches!(result, Err(EventError::MissingTopic(1))));
    }

    /// PheromoneDeposited with correct encoding is processed.
    #[test]
    fn pheromone_deposited_roundtrip() {
        let mut index = OnChainHdcIndex::new();
        let pheromone_id = B256::from([0x01; 32]);
        let location = B256::from([0x02; 32]);
        let mut depositor_topic = B256::ZERO;
        depositor_topic[12..32].copy_from_slice(&[0x03; 20]);

        let topics = vec![*topics::PHEROMONE_DEPOSITED, pheromone_id, location, depositor_topic];

        // Encode data: pType=1, intensity=42, blockNumber=100
        let mut data = vec![0u8; 96]; // 3 words
        data[31] = 1; // pType
        let intensity_bytes = 42u64.to_be_bytes();
        data[56..64].copy_from_slice(&intensity_bytes); // intensity in word 1
        let block_bytes = 100u64.to_be_bytes();
        data[88..96].copy_from_slice(&block_bytes); // blockNumber in word 2

        let result = process_log(&mut index, &topics, &data, &Address::ZERO);
        assert!(result.is_ok());
    }

    /// InsightPurged transitions an insight to Expired.
    #[test]
    fn insight_purged_marks_expired() {
        let mut index = OnChainHdcIndex::new();
        let v = kora_hdc::HdcVector::random(1);
        let publisher = Address::ZERO;
        let id = index.insert_insight(v, publisher, 100).unwrap();
        index.update_state(&id, InsightState::Accepted).unwrap();

        let mut purger_topic = B256::ZERO;
        purger_topic[12..32].copy_from_slice(&[0x01; 20]);

        let topics = vec![*topics::INSIGHT_PURGED, id, purger_topic];
        let result = process_log(&mut index, &topics, &[], &Address::ZERO);
        assert!(result.is_ok());

        // Verify it's no longer searchable.
        let v2 = kora_hdc::HdcVector::random(1);
        let results = index.search(&v2, 10);
        assert!(results.is_empty());
    }

    /// InsightStateChanged decodes and applies the new state.
    #[test]
    fn insight_state_changed_applies_new_state() {
        let mut index = OnChainHdcIndex::new();
        let v = kora_hdc::HdcVector::random(2);
        let publisher = Address::ZERO;
        let id = index.insert_insight(v.clone(), publisher, 50).unwrap();

        // Transition from Submitted (1) to Accepted (4).
        let topics = vec![*topics::INSIGHT_STATE_CHANGED, id];
        let mut data = vec![0u8; 64];
        data[31] = 1; // oldState = Submitted
        data[63] = 4; // newState = Accepted

        let result = process_log(&mut index, &topics, &data, &Address::ZERO);
        assert!(result.is_ok());

        // Should now appear in search.
        let results = index.search(&v, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, id);
    }

    /// Invalid state value returns an error.
    #[test]
    fn invalid_state_value_errors() {
        let mut index = OnChainHdcIndex::new();
        let v = kora_hdc::HdcVector::random(3);
        let publisher = Address::ZERO;
        let id = index.insert_insight(v, publisher, 50).unwrap();

        let topics = vec![*topics::INSIGHT_STATE_CHANGED, id];
        let mut data = vec![0u8; 64];
        data[31] = 0;
        data[63] = 99; // invalid state

        let result = process_log(&mut index, &topics, &data, &Address::ZERO);
        assert!(matches!(result, Err(EventError::InvalidEnumValue { .. })));
    }
}
