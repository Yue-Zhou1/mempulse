use bench::run_simulation_roundtrip_once;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;

fn simulation_roundtrip_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("simulation_roundtrip");
    for candidate_count in [32_usize, 128, 256] {
        group.throughput(Throughput::Elements(candidate_count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(candidate_count),
            &candidate_count,
            |b, input| {
                b.iter(|| {
                    black_box(run_simulation_roundtrip_once(black_box(*input)).handoff_total)
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, simulation_roundtrip_bench);
criterion_main!(benches);
