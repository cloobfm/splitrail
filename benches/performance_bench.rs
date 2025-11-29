use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId, Throughput};
use splitrail_dashboard::analyzer::{Analyzer, AnalyzerRegistry};
use splitrail_dashboard::analyzers::*;
use splitrail_dashboard::types::{ConversationMessage, DailyStats, Stats, Application, MessageRole};
use splitrail_dashboard::utils::{hash_text, aggregate_by_date};
use std::time::Duration;
use std::collections::BTreeMap;
use chrono::{DateTime, Utc};
use tempfile::TempDir;
use std::fs::File;
use std::io::Write;

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

// Benchmark message deduplication (CRITICAL BOTTLENECK)
fn bench_deduplication(c: &mut Criterion) {
    let mut group = c.benchmark_group("deduplication");

    // Test hash computation performance
    group.bench_function("hash_computation", |b| {
        b.iter(|| {
            let text = "sample text for hashing with more content to simulate real messages";
            let _ = black_box(hash_text(black_box(text)));
        });
    });

    // Test hash computation with different text sizes
    for size in [100, 1000, 10000, 100000].iter() {
        group.throughput(Throughput::Bytes(*size as u64));
        group.bench_with_input(
            BenchmarkId::new("hash_computation_by_size", size),
            size,
            |b, &size| {
                let text = "x".repeat(size);
                b.iter(|| {
                    let _ = black_box(hash_text(black_box(&text)));
                });
            },
        );
    }

    // Test O(n²) deduplication with different message counts
    for msg_count in [100, 500, 1000, 5000].iter() {
        group.throughput(Throughput::Elements(*msg_count as u64));
        group.bench_with_input(
            BenchmarkId::new("current_deduplication", msg_count),
            msg_count,
            |b, &msg_count| {
                let messages = create_test_messages(msg_count);
                b.iter(|| {
                    let _ = black_box(deduplicate_messages_by_local_hash_current(black_box(messages.clone())));
                });
            },
        );
    }

    group.finish();
}

// Benchmark file parsing performance
fn bench_file_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("file_parsing");
    
    let temp_dir = TempDir::new().unwrap();
    
    // Test JSONL parsing with different file sizes
    for line_count in [100, 1000, 10000].iter() {
        group.throughput(Throughput::Elements(*line_count as u64));
        group.bench_with_input(
            BenchmarkId::new("jsonl_parsing", line_count),
            line_count,
            |b, &line_count| {
                let test_file = create_test_jsonl_file(&temp_dir, line_count);
                b.iter(|| {
                    let _ = black_box(parse_jsonl_file_bench(&test_file));
                });
            },
        );
    }

    // Test parallel vs sequential parsing
    let test_files = create_multiple_test_files(&temp_dir, 10, 1000);
    
    group.bench_function("sequential_parsing", |b| {
        b.iter(|| {
            let _ = black_box(parse_files_sequential(&test_files));
        });
    });

    group.bench_function("parallel_parsing", |b| {
        b.iter(|| {
            let _ = black_box(parse_files_parallel(&test_files));
        });
    });

    group.finish();
}

// Benchmark stats aggregation
fn bench_stats_aggregation_detailed(c: &mut Criterion) {
    let mut group = c.benchmark_group("stats_aggregation_detailed");

    // Test aggregation with different message counts
    for msg_count in [1000, 5000, 10000, 50000].iter() {
        group.throughput(Throughput::Elements(*msg_count as u64));
        group.bench_with_input(
            BenchmarkId::new("aggregate_by_date", msg_count),
            msg_count,
            |b, &msg_count| {
                let messages = create_test_messages(msg_count);
                b.iter(|| {
                    let _ = black_box(aggregate_by_date(black_box(&messages)));
                });
            },
        );
    }

    // Test BTreeMap operations
    group.bench_function("btreemap_insertions", |b| {
        b.iter(|| {
            let mut map: BTreeMap<String, DailyStats> = BTreeMap::new();
            for i in 0..1000 {
                let date = format!("2024-01-{:02}", (i % 30) + 1);
                map.insert(date, create_test_daily_stats());
            }
            black_box(map)
        });
    });

    group.finish();
}

// Benchmark memory allocation patterns
fn bench_memory_patterns(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_patterns");

    // Test Vec growth patterns
    group.bench_function("vec_without_capacity", |b| {
        b.iter(|| {
            let mut vec = Vec::new();
            for i in 0..10000 {
                vec.push(i);
            }
            black_box(vec)
        });
    });

    group.bench_function("vec_with_capacity", |b| {
        b.iter(|| {
            let mut vec = Vec::with_capacity(10000);
            for i in 0..10000 {
                vec.push(i);
            }
            black_box(vec)
        });
    });

    // Test string cloning vs Cow
    group.bench_function("string_cloning", |b| {
        let text = "test string content".to_string();
        b.iter(|| {
            let _ = black_box(text.clone());
        });
    });

    group.finish();
}

// Benchmark TUI rendering performance
fn bench_tui_performance(c: &mut Criterion) {
    let mut group = c.benchmark_group("tui_performance");

    let stats = create_test_analyzer_stats();
    
    // Test sparkline cache building
    group.bench_function("sparkline_cache_building", |b| {
        b.iter(|| {
            let _ = black_box(build_activity_sparkline_cache_bench(&stats));
        });
    });

    // Test tokens per second calculation
    group.bench_function("tokens_per_second_calc", |b| {
        b.iter(|| {
            let _ = black_box(calculate_tokens_per_second_bench(&stats));
        });
    });

    group.finish();
}

// Helper functions for benchmark data generation
fn create_test_messages(count: usize) -> Vec<ConversationMessage> {
    let mut messages = Vec::with_capacity(count);
    for i in 0..count {
        messages.push(ConversationMessage {
            application: Application::ClaudeCode,
            date: Utc::now(),
            project_hash: format!("project_{}", i % 100),
            conversation_hash: format!("conv_{}", i % 50),
            local_hash: Some(format!("local_hash_{}", i % 20)), // Some duplicates for deduplication testing
            global_hash: format!("global_hash_{}", i),
            model: Some("claude-3-sonnet-20240229".to_string()),
            stats: Stats {
                input_tokens: 1000 + (i as u64 % 5000),
                output_tokens: 500 + (i as u64 % 2000),
                cost: 0.01 + (i as f64 * 0.001),
                tool_calls: (i % 10) as u32,
                ..Default::default()
            },
            role: if i % 2 == 0 { MessageRole::User } else { MessageRole::Assistant },
            content: Some(format!("Test message content {}", i)),
        });
    }
    messages
}

fn create_test_daily_stats() -> DailyStats {
    DailyStats {
        date: "2024-01-01".to_string(),
        user_messages: 100,
        ai_messages: 100,
        conversations: 50,
        models: BTreeMap::from([
            ("claude-3-sonnet".to_string(), 75),
            ("claude-3-haiku".to_string(), 25),
        ]),
        stats: Stats {
            input_tokens: 100000,
            output_tokens: 50000,
            cost: 5.0,
            tool_calls: 200,
            ..Default::default()
        },
    }
}

fn create_test_analyzer_stats() -> Vec<splitrail_dashboard::types::AgenticCodingToolStats> {
    vec![
        splitrail_dashboard::types::AgenticCodingToolStats {
            analyzer_name: "Claude Code".to_string(),
            num_conversations: 1000,
            daily_stats: BTreeMap::from([
                ("2024-01-01".to_string(), create_test_daily_stats()),
                ("2024-01-02".to_string(), create_test_daily_stats()),
            ]),
            messages: create_test_messages(2000),
        }
    ]
}

fn create_test_jsonl_file(temp_dir: &TempDir, line_count: usize) -> std::path::PathBuf {
    let file_path = temp_dir.path().join("test.jsonl");
    let mut file = File::create(&file_path).unwrap();
    
    for i in 0..line_count {
        let json_line = serde_json::json!({
            "type": "message",
            "message": {
                "id": format!("msg_{}", i),
                "content": format!("Test message {}", i),
                "timestamp": "2024-01-01T00:00:00Z"
            }
        });
        writeln!(file, "{}", json_line).unwrap();
    }
    
    file_path
}

fn create_multiple_test_files(temp_dir: &TempDir, file_count: usize, lines_per_file: usize) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for i in 0..file_count {
        let file_path = temp_dir.path().join(format!("test_{}.jsonl", i));
        let mut file = File::create(&file_path).unwrap();
        
        for j in 0..lines_per_file {
            let json_line = serde_json::json!({
                "type": "message",
                "message": {
                    "id": format!("msg_{}_{}", i, j),
                    "content": format!("Test message {} from file {}", j, i),
                    "timestamp": "2024-01-01T00:00:00Z"
                }
            });
            writeln!(file, "{}", json_line).unwrap();
        }
        files.push(file_path);
    }
    files
}

// Current (inefficient) deduplication implementation for comparison
fn deduplicate_messages_by_local_hash_current(messages: Vec<ConversationMessage>) -> Vec<ConversationMessage> {
    let mut result = Vec::new();
    let mut seen_hashes = std::collections::HashSet::new();
    
    for msg in messages {
        if let Some(local_hash) = &msg.local_hash {
            if !seen_hashes.contains(local_hash) {
                seen_hashes.insert(local_hash.clone());
                result.push(msg);
            }
        } else {
            result.push(msg);
        }
    }
    
    result
}

fn parse_jsonl_file_bench(file_path: &std::path::Path) -> Vec<serde_json::Value> {
    use std::io::{BufRead, BufReader};
    let file = File::open(file_path).unwrap();
    let reader = BufReader::new(file);
    
    reader.lines()
        .filter_map(|line| line.ok())
        .filter_map(|line| serde_json::from_str(&line).ok())
        .collect()
}

fn parse_files_sequential(files: &[std::path::PathBuf]) -> Vec<serde_json::Value> {
    let mut all_values = Vec::new();
    for file_path in files {
        all_values.extend(parse_jsonl_file_bench(file_path));
    }
    all_values
}

fn parse_files_parallel(files: &[std::path::PathBuf]) -> Vec<serde_json::Value> {
    use rayon::prelude::*;
    files.par_iter()
        .flat_map(|file_path| parse_jsonl_file_bench(file_path))
        .collect()
}

fn build_activity_sparkline_cache_bench(
    stats: &[splitrail_dashboard::types::AgenticCodingToolStats],
) -> std::collections::HashMap<String, Vec<ratatui::text::Span<'static>>> {
    // Simplified version for benchmarking
    let mut cache = std::collections::HashMap::new();
    for analyzer_stats in stats {
        let spans = vec![
            ratatui::text::Span::raw("█"),
            ratatui::text::Span::raw("▄"),
            ratatui::text::Span::raw(" "),
        ];
        cache.insert(analyzer_stats.analyzer_name.clone(), spans);
    }
    cache
}

fn calculate_tokens_per_second_bench(
    stats: &[splitrail_dashboard::types::AgenticCodingToolStats],
) -> f64 {
    let mut total_tokens = 0u64;
    for analyzer_stats in stats {
        for daily_stats in analyzer_stats.daily_stats.values() {
            total_tokens += daily_stats.stats.output_tokens;
        }
    }
    total_tokens as f64 / 900.0 // 15 minutes in seconds
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
    bench_deduplication,
    bench_file_parsing,
    bench_stats_aggregation_detailed,
    bench_memory_patterns,
    bench_tui_performance
);
criterion_main!(benches);
