use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use splitrail_dashboard::analyzer::{Analyzer, AnalyzerRegistry};
use splitrail_dashboard::analyzers::*;
use std::time::Duration;

// Benchmark initial data loading from all analyzers
fn bench_initial_load(c: &mut Criterion) {
    let mut group = c.benchmark_group("initial_load");
    group.measurement_time(Duration::from_secs(10));

    let runtime = tokio::runtime::Runtime::new().unwrap();

    // Test each analyzer individually
    for analyzer_name in &["Claude Code", "Kilo Code", "Codex CLI", "WARP"] {
        group.bench_with_input(
            BenchmarkId::from_parameter(analyzer_name),
            analyzer_name,
            |b, &name| {
                b.to_async(&runtime).iter(|| async {
                    let analyzer: Box<dyn Analyzer> = match name {
                        "Claude Code" => Box::new(ClaudeCodeAnalyzer::new()),
                        "Kilo Code" => Box::new(KiloCodeAnalyzer::new()),
                        "Codex CLI" => Box::new(CodexCliAnalyzer::new()),
                        "WARP" => Box::new(WarpDevAnalyzer::new()),
                        _ => panic!("Unknown analyzer"),
                    };

                    if analyzer.is_available() {
                        let _ = black_box(analyzer.get_stats().await);
                    }
                });
            },
        );
    }

    // Test full registry load
    group.bench_function("full_registry", |b| {
        b.to_async(&runtime).iter(|| async {
            let registry = create_test_registry();
            let _ = black_box(registry.load_all_stats().await);
        });
    });

    group.finish();
}

// Benchmark stats aggregation
fn bench_stats_aggregation(c: &mut Criterion) {
    let mut group = c.benchmark_group("stats_aggregation");

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let registry = create_test_registry();
    let stats = runtime.block_on(async { registry.load_all_stats().await.unwrap() });

    group.bench_function("aggregate_by_date", |b| {
        b.iter(|| {
            for analyzer_stats in &stats.analyzer_stats {
                let _ = black_box(splitrail_dashboard::utils::aggregate_by_date(&analyzer_stats.messages));
            }
        });
    });

    group.finish();
}

// Benchmark message deduplication
fn bench_deduplication(c: &mut Criterion) {
    let mut group = c.benchmark_group("deduplication");

    let runtime = tokio::runtime::Runtime::new().unwrap();

    group.bench_function("hash_computation", |b| {
        b.iter(|| {
            let text = "sample text for hashing";
            let _ = black_box(splitrail_dashboard::utils::hash_text(text));
        });
    });

    group.finish();
}

fn create_test_registry() -> AnalyzerRegistry {
    let mut registry = AnalyzerRegistry::new();
    registry.register(ClaudeCodeAnalyzer::new());
    registry.register(KiloCodeAnalyzer::new());
    registry.register(CodexCliAnalyzer::new());
    registry.register(WarpDevAnalyzer::new());
    registry.register(GeminiCliAnalyzer::new());
    registry.register(ClineAnalyzer::new());
    registry
}

criterion_group!(
    benches,
    bench_initial_load,
    bench_stats_aggregation,
    bench_deduplication
);
criterion_main!(benches);
