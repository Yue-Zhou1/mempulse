use bench::{run_pipeline_once, synthetic_batch};
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;

fn pipeline_latency_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("pipeline_latency");
    for batch_size in [64_usize, 256, 1024] {
        let batch = synthetic_batch(batch_size);
        group.throughput(Throughput::Elements(batch_size as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            &batch,
            |b, input| {
                b.iter(|| black_box(run_pipeline_once(black_box(input))));
            },
        );
    }
    group.finish();
}

criterion_group!(benches, pipeline_latency_bench);
criterion_main!(benches);
