//! Integration tests for `HDCState::on_finalize_extend` — the
//! deterministic event-replay hook the spec-aligned HDC precompile uses
//! to keep validator state convergent.
//!
//! Spec source:
//!   `~/obsidian-vault/research/2026-05-04-wp-agent-chainv2/raw/02-daeji/02-precompiles-and-contracts.md`
//!   lines 173-178: "in-memory `HdcIndex` rebuilt at block boundaries from
//!   the latest `InsightBoard` state and `InsightPosted` event content".

use alloy_primitives::{Address, B256, Bytes, LogData, U256, address, keccak256};
use alloy_sol_types::SolEvent;
use kora_precompiles::{
    HDCState,
    insight_event::{InsightPosted, decode_insight_posted, insight_posted_topic0},
};

const INSIGHT_BOARD: Address = address!("0x000000000000000000000000000000000000000a");

/// Build a synthetic `InsightPosted` log fully ABI-correct.
fn synth_log(id: u64, kind: u8, posted_at: u64, hdc_vector: Vec<u8>) -> LogData {
    let topic0 = insight_posted_topic0();
    let topic_id = B256::from(U256::from(id));
    let topic_poster =
        B256::from(address!("0x00000000000000000000000000000000beefBeEF").into_word());
    let topic_kind = B256::from(U256::from(kind));

    let body = InsightPosted {
        id: U256::from(id),
        poster: address!("0x00000000000000000000000000000000beefBeEF"),
        kind,
        contentHash: B256::ZERO,
        hdcFingerprint: keccak256(&hdc_vector),
        postedAt: posted_at,
        revealAt: 0,
        hdcVector: Bytes::from(hdc_vector),
        uri: "ipfs://abc".to_string(),
    };
    let data = body.encode_data();
    LogData::new(vec![topic0, topic_id, topic_poster, topic_kind], Bytes::from(data))
        .expect("topic count valid")
}

#[test]
fn on_finalize_extend_indexes_synthetic_logs() {
    let state = HDCState::new();
    let block_ts = 1_000_000;

    let logs = vec![
        synth_log(1, 0, block_ts, vec![0xAA; 1280]),
        synth_log(2, 0, block_ts, vec![0xBB; 1280]),
        synth_log(3, 2, block_ts, vec![0xCC; 1280]), // Warning
    ];
    let events: Vec<_> =
        logs.iter().filter_map(|log| decode_insight_posted(INSIGHT_BOARD, log)).collect();
    assert_eq!(events.len(), 3);

    state.on_finalize_extend(block_ts, INSIGHT_BOARD, events);

    assert_eq!(state.index.read().len(), 3);
}

#[test]
fn on_finalize_extend_filters_logs_from_other_emitters() {
    let state = HDCState::new();
    let block_ts = 1_000_000;
    let other = address!("0x000000000000000000000000000000000000bbbb");

    let log = synth_log(1, 0, block_ts, vec![0xAA; 1280]);
    let event_from_other = decode_insight_posted(other, &log).expect("decode");
    let event_from_canonical = decode_insight_posted(INSIGHT_BOARD, &log).expect("decode");

    state.on_finalize_extend(block_ts, INSIGHT_BOARD, [event_from_other, event_from_canonical]);

    // Only the canonical emitter's event should land in the index.
    assert_eq!(state.index.read().len(), 1);
}

#[test]
fn on_finalize_extend_evicts_expired_warning_after_21_minutes() {
    let state = HDCState::new();
    let post_ts = 1_000_000;
    let after_dead_ts = post_ts + 22 * 60; // 22 minutes — past 7×18s? No: 22 min = 1320 s, 7×18=126s, so well past dead.

    // Post a Warning (Tier::Transient default ⇒ effective hl = 18s).
    let log = synth_log(99, 2, post_ts, vec![0xCC; 1280]);
    let event = decode_insight_posted(INSIGHT_BOARD, &log).expect("decode");
    state.on_finalize_extend(post_ts, INSIGHT_BOARD, [event]);
    assert_eq!(state.index.read().len(), 1);

    // Replay an empty log set at a later timestamp; eviction runs even when
    // there are no new events.
    state.on_finalize_extend(after_dead_ts, INSIGHT_BOARD, []);
    assert_eq!(
        state.index.read().len(),
        0,
        "Warning should be evicted at 22 minutes (effective half-life is 18s; 7×18=126s past)"
    );
}

#[test]
fn on_finalize_extend_keeps_long_lived_kinds_alive() {
    let state = HDCState::new();
    let post_ts = 1_000_000;
    // 1 hour later — well past Warning's death at 126s but well before
    // Insight's death (Insight Transient effective hl = 7d/10 = 60480s;
    // dead at 7 × 60480 = ~4.9d).
    let later_ts = post_ts + 60 * 60;

    let log = synth_log(1, 0, post_ts, vec![0xDD; 1280]); // Insight kind
    let event = decode_insight_posted(INSIGHT_BOARD, &log).expect("decode");
    state.on_finalize_extend(post_ts, INSIGHT_BOARD, [event]);

    // Replay empty events at later timestamp — eviction should NOT drop
    // the Insight (still well within its lifespan).
    state.on_finalize_extend(later_ts, INSIGHT_BOARD, []);
    assert_eq!(state.index.read().len(), 1);
}

#[test]
fn on_finalize_extend_is_deterministic_across_replays() {
    // The whole point of the rework: two validators executing the same
    // sequence of events must end up with identical index state. We
    // simulate by running the same log batch through two HDCState
    // instances and comparing.
    let state_a = HDCState::new();
    let state_b = HDCState::new();
    let block_ts = 1_000_000;

    let logs = vec![
        synth_log(10, 0, block_ts, vec![0x01; 1280]),
        synth_log(20, 1, block_ts, vec![0x02; 1280]),
        synth_log(30, 4, block_ts, vec![0x03; 1280]),
    ];
    let events_a: Vec<_> =
        logs.iter().filter_map(|log| decode_insight_posted(INSIGHT_BOARD, log)).collect();
    let events_b: Vec<_> =
        logs.iter().filter_map(|log| decode_insight_posted(INSIGHT_BOARD, log)).collect();

    state_a.on_finalize_extend(block_ts, INSIGHT_BOARD, events_a);
    state_b.on_finalize_extend(block_ts, INSIGHT_BOARD, events_b);

    let idx_a = state_a.index.read();
    let idx_b = state_b.index.read();
    assert_eq!(idx_a.len(), idx_b.len());

    let entries_a: Vec<_> = idx_a.iter().map(|e| (e.id, e.posted_at)).collect();
    let entries_b: Vec<_> = idx_b.iter().map(|e| (e.id, e.posted_at)).collect();
    assert_eq!(entries_a, entries_b);
}

#[test]
fn on_finalize_extend_skips_malformed_events_gracefully() {
    let state = HDCState::new();
    let block_ts = 1_000_000;

    // Vector with wrong length should be silently skipped (not panic).
    let log_bad = synth_log(1, 0, block_ts, vec![0xAA; 100]); // wrong length
    let log_good = synth_log(2, 0, block_ts, vec![0xBB; 1280]);

    let events: Vec<_> = [log_bad, log_good]
        .iter()
        .filter_map(|log| decode_insight_posted(INSIGHT_BOARD, log))
        .collect();

    state.on_finalize_extend(block_ts, INSIGHT_BOARD, events);

    // Only the well-formed log should land in the index.
    assert_eq!(state.index.read().len(), 1);
}
