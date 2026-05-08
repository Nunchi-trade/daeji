use criterion::{Criterion, black_box, criterion_group, criterion_main};
use kora_hdc::{
    HdcVector, bind,
    bundle::{BundleAccumulator, bundle},
    deserialize, hamming_distance, permute,
    search::{BruteForceIndex, SearchIndex},
    serialize,
};

fn bench_hamming_distance(c: &mut Criterion) {
    let a = HdcVector::random(42);
    let b = HdcVector::random(43);

    c.bench_function("hamming_distance", |bencher| {
        bencher.iter(|| black_box(hamming_distance(&a, &b)))
    });
}

fn bench_bind(c: &mut Criterion) {
    let a = HdcVector::random(42);
    let b = HdcVector::random(43);

    c.bench_function("bind", |bencher| bencher.iter(|| black_box(bind(&a, &b))));
}

fn bench_bundle_50(c: &mut Criterion) {
    let vecs: Vec<HdcVector> = (0..50).map(|i| HdcVector::random(i)).collect();
    let refs: Vec<&HdcVector> = vecs.iter().collect();

    c.bench_function("bundle_50", |bencher| bencher.iter(|| black_box(bundle(&refs))));
}

fn bench_permute(c: &mut Criterion) {
    let v = HdcVector::random(42);

    c.bench_function("permute_37", |bencher| bencher.iter(|| black_box(permute(&v, 37))));
}

fn bench_brute_force_search_1000(c: &mut Criterion) {
    let mut index = BruteForceIndex::new();
    for i in 0..1000u64 {
        let v = HdcVector::random(i);
        let k = kora_hdc::vector_id(&v);
        index.insert(k, v).unwrap();
    }
    let query = HdcVector::random(999999);

    c.bench_function("brute_force_search_1000", |bencher| {
        bencher.iter(|| black_box(index.search(&query, 10)))
    });
}

fn bench_serialize_roundtrip(c: &mut Criterion) {
    let v = HdcVector::random(42);

    c.bench_function("serialize_roundtrip", |bencher| {
        bencher.iter(|| {
            let bytes = serialize(&v);
            black_box(deserialize(&bytes))
        })
    });
}

criterion_group!(
    benches,
    bench_hamming_distance,
    bench_bind,
    bench_bundle_50,
    bench_permute,
    bench_brute_force_search_1000,
    bench_serialize_roundtrip,
);
criterion_main!(benches);
