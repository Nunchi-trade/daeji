//! Integration tests for HDC knowledge store.

use kora_hdc::{
    HdcVector,
    knowledge::{KnowledgeEntry, KnowledgeKind, KnowledgeSource, KnowledgeStore, KnowledgeTier},
};

fn make_store() -> KnowledgeStore {
    // 400ms ticks -> 9000 ticks = 1 hour
    KnowledgeStore::new(400)
}

fn make_entry(seed: u64, tick: u64) -> KnowledgeEntry {
    KnowledgeEntry::new(
        HdcVector::random(seed),
        KnowledgeKind::Insight,
        format!("entry-{seed}"),
        KnowledgeSource::SelfDerived,
        tick,
    )
}

#[test]
fn knowledge_decay_reduces_balance() {
    let mut store = make_store();
    let entry = make_entry(42, 0);
    let key = entry.key;
    store.insert(entry);

    let balance_before = store.get(&key).unwrap().balance;
    assert!((balance_before - 1.0).abs() < 1e-10, "initial balance should be 1.0");

    // Insight at Transient: effective half-life = 72 * 0.1 = 7.2 hours
    // 7.2 hours at 400ms ticks = 7.2 * 9000 = 64800 ticks
    store.tick(64_800);

    let balance_after = store.get(&key).unwrap().balance;
    assert!(
        balance_after < balance_before,
        "balance should decrease: {balance_before} -> {balance_after}"
    );
    // After one half-life, balance should be ~0.5
    assert!(
        (balance_after - 0.5).abs() < 0.1,
        "after one half-life, balance should be ~0.5, got {balance_after}"
    );
}

#[test]
fn tier_promotion_transient_to_working_to_consolidated() {
    let mut store = make_store();
    let entry = make_entry(1, 0);
    let key = entry.key;
    store.insert(entry);

    // Initially: Transient tier
    assert_eq!(store.get(&key).unwrap().tier, KnowledgeTier::Transient);

    // Reinforce 3 times -> Working
    for i in 1..=3 {
        store.reinforce(&key, i, 0.5);
    }
    store.tick(4);
    assert_eq!(store.get(&key).unwrap().tier, KnowledgeTier::Working);

    // Reinforce to 10 total -> Consolidated
    for i in 4..=10 {
        store.reinforce(&key, i, 0.5);
    }
    store.tick(11);
    assert_eq!(store.get(&key).unwrap().tier, KnowledgeTier::Consolidated);
}
