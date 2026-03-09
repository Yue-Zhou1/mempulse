use bench::run_storage_snapshot_once;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;

fn storage_snapshot_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("storage_snapshot");
    for (pending_count, tail_event_count) in [(128_usize, 16_usize), (512, 32)] {
        group.throughput(Throughput::Elements(pending_count as u64));
        group.bench_with_input(
            BenchmarkId::new(
                "pending_tail",
                format!("{pending_count}_{tail_event_count}"),
            ),
            &(pending_count, tail_event_count),
            |b, input| {
                b.iter(|| {
                    black_box(
                        run_storage_snapshot_once(black_box(input.0), black_box(input.1))
                            .replay_event_count,
                    )
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, storage_snapshot_bench);
criterion_main!(benches);
