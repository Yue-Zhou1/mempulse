use bench::run_scheduler_pipeline_once;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;

fn scheduler_pipeline_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("scheduler_pipeline");
    for batch_size in [64_usize, 256, 512] {
        group.throughput(Throughput::Elements(batch_size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(batch_size), &batch_size, |b, input| {
            b.iter(|| black_box(run_scheduler_pipeline_once(black_box(*input)).ready_total));
        });
    }
    group.finish();
}

criterion_group!(benches, scheduler_pipeline_bench);
criterion_main!(benches);
