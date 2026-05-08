//! Stress tests for HDC knowledge store.

use kora_hdc::{
    HdcVector,
    cognitive::affect::PadState,
    knowledge::{KnowledgeEntry, KnowledgeKind, KnowledgeSource, KnowledgeStore, RetrievalContext},
};

fn make_store() -> KnowledgeStore {
    KnowledgeStore::new(400)
}

#[test]
fn stress_10k_knowledge_entries() {
    let mut store = make_store();
    let ctx =
        RetrievalContext { current_tick: 0, mood: PadState::neutral(), tick_duration_ms: 400 };

    // Insert 10,000 entries
    let mut keys = Vec::with_capacity(10_000);
    for i in 0..10_000u64 {
        let v = HdcVector::random(i);
        let entry = KnowledgeEntry::new(
            v,
            KnowledgeKind::Insight,
            format!("entry-{i}"),
            KnowledgeSource::SelfDerived,
            0,
        );
        keys.push(entry.key);
        store.insert(entry);
    }

    assert_eq!(store.len(), 10_000);

    // Search should still work correctly
    let query = HdcVector::random(5000);
    let results = store.search(&query, 10, &ctx);

    // The query vector itself should be the top result (distance = 0)
    assert_eq!(results[0].hamming_distance, 0);
}
