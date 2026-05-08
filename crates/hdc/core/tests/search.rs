//! Integration tests for HDC search indices.

use kora_hdc::{
    HdcVector,
    search::{BruteForceIndex, H256, SearchIndex},
    vector_id,
};

fn make_key(i: u64) -> H256 {
    let v = HdcVector::random(i);
    vector_id(&v)
}

#[test]
fn brute_force_insert_1000_search_top_k() {
    let mut index = BruteForceIndex::new();

    let vectors: Vec<(H256, HdcVector)> = (0..1000u64)
        .map(|i| {
            let v = HdcVector::random(i);
            let k = vector_id(&v);
            (k, v)
        })
        .collect();

    for (k, v) in &vectors {
        index.insert(*k, v.clone()).unwrap();
    }

    // Search for each of the first 10 vectors; they should be their own top-1
    for (k, v) in vectors.iter().take(10) {
        let results = index.search(v, 5).unwrap();
        assert_eq!(results[0].0, *k, "top-1 for vector should be itself");
        assert_eq!(results[0].1, 0, "distance to self should be 0");
    }
}

#[test]
fn brute_force_1000_results_sorted() {
    let mut index = BruteForceIndex::new();

    for i in 0..1000u64 {
        let v = HdcVector::random(i);
        let k = vector_id(&v);
        index.insert(k, v).unwrap();
    }

    let query = HdcVector::random(42);
    let results = index.search(&query, 20).unwrap();

    // Verify sorted by distance ascending
    for w in results.windows(2) {
        assert!(w[0].1 <= w[1].1, "results should be sorted by distance");
    }
}
