use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId, Throughput};
use splitrail_dashboard::types::{ConversationMessage, Application, MessageRole};
use splitrail_dashboard::utils::aggregate_by_date;
use std::time::Duration;
use tempfile::TempDir;
use std::fs::File;
use std::io::Write;

// Benchmark the ACTUAL bottlenecks we suspect
fn bench_real_bottlenecks(c: &mut Criterion) {
    let mut group = c.benchmark_group("real_bottlenecks");
    group.measurement_time(Duration::from_secs(5));

    // Test 1: JSON parsing performance (major suspect)
    let temp_dir = TempDir::new().unwrap();
    let test_files = create_test_jsonl_files(&temp_dir, &[100, 1000, 5000]);
    
    for (file, line_count) in test_files.iter() {
        group.throughput(Throughput::Bytes(file.metadata().unwrap().len() as u64));
        group.bench_with_input(
            BenchmarkId::new("json_parsing", line_count),
            line_count,
            |b, &_line_count| {
                b.iter(|| {
                    let _ = black_box(parse_jsonl_file_realistic(black_box(file)));
                });
            },
        );
    }

    // Test 2: File I/O only (no parsing)
    for (file, line_count) in test_files.iter() {
        group.bench_with_input(
            BenchmarkId::new("file_read_only", line_count),
            line_count,
            |b, &_line_count| {
                b.iter(|| {
                    let _ = black_box(std::fs::read_to_string(black_box(file)).unwrap());
                });
            },
        );
    }

    // Test 3: Aggregation performance
    let message_sets = create_message_sets(&[1000, 5000, 10000]);
    for (msg_count, messages) in message_sets.iter() {
        group.throughput(Throughput::Elements(*msg_count as u64));
        group.bench_with_input(
            BenchmarkId::new("aggregation", msg_count),
            msg_count,
            |b, &_msg_count| {
                b.iter(|| {
                    let _ = black_box(aggregate_by_date(black_box(messages)));
                });
            },
        );
    }

    // Test 4: String operations (UI formatting)
    let test_strings = create_test_strings(&[50, 500, 5000]);
    for (len, string) in test_strings.iter() {
        group.throughput(Throughput::Bytes(*len as u64));
        group.bench_with_input(
            BenchmarkId::new("string_formatting", len),
            len,
            |b, &_len| {
                b.iter(|| {
                    // Simulate UI string formatting
                    let _ = black_box(format!("{}: {} tokens", 
                        black_box(&string[..std::cmp::min(20, string.len())]), 
                        black_box(1234567)
                    ));
                });
            },
        );
    }

    group.finish();
}

// Benchmark file system operations
fn bench_file_system_ops(c: &mut Criterion) {
    let mut group = c.benchmark_group("file_system_ops");
    
    let temp_dir = TempDir::new().unwrap();
    
    // Test glob pattern matching (file discovery)
    group.bench_function("glob_pattern_matching", |b| {
        b.iter(|| {
            let _ = black_box(glob::glob("/Users/bzl/.claude/projects/*/conversations_*.jsonl")
                .unwrap()
                .count());
        });
    });
    
    // Test file metadata operations
    let test_file = temp_dir.path().join("test.jsonl");
    std::fs::write(&test_file, "test content").unwrap();
    
    group.bench_function("file_metadata", |b| {
        b.iter(|| {
            let _ = black_box(test_file.metadata().unwrap());
            let _ = black_box(test_file.is_file());
            let _ = black_box(test_file.metadata().unwrap().modified().unwrap());
        });
    });
    
    group.finish();
}

// Benchmark memory allocation patterns
fn bench_memory_patterns(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_patterns_real");
    
    // Test Vec growth patterns (realistic sizes)
    for size in [1000, 5000, 10000].iter() {
        group.bench_with_input(
            BenchmarkId::new("vec_growth_messages", size),
            size,
            |b, &size| {
                b.iter(|| {
                    let mut vec: Vec<ConversationMessage> = Vec::new();
                    for i in 0..size {
                        vec.push(create_test_message(i));
                    }
                    black_box(vec)
                });
            },
        );
    }
    
    // Test string cloning (common in message processing)
    let test_string = "This is a test message content that simulates real message data with some length to it".to_string();
    
    group.bench_function("string_cloning_hot", |b| {
        b.iter(|| {
            for _ in 0..1000 {
                let _ = black_box(test_string.clone());
            }
        });
    });
    
    group.finish();
}

// Helper functions to create realistic test data

fn create_test_jsonl_files(temp_dir: &TempDir, line_counts: &[usize]) -> Vec<(std::path::PathBuf, usize)> {
    let mut files = Vec::new();
    
    for &line_count in line_counts {
        let file_path = temp_dir.path().join(format!("test_{}.jsonl", line_count));
        let mut file = File::create(&file_path).unwrap();
        
        for i in 0..line_count {
            let json_line = serde_json::json!({
                "type": "message",
                "message": {
                    "id": format!("msg_{}", i),
                    "content": format!("Test message {} with some realistic content that simulates actual Claude Code conversations", i),
                    "timestamp": "2024-01-01T00:00:00Z",
                    "role": if i % 2 == 0 { "user" } else { "assistant" },
                    "model": "claude-3-sonnet-20240229",
                    "usage": {
                        "input_tokens": 1000 + (i % 5000),
                        "output_tokens": 500 + (i % 2000)
                    }
                }
            });
            writeln!(file, "{}", json_line).unwrap();
        }
        
        files.push((file_path, line_count));
    }
    
    files
}

fn create_message_sets(counts: &[usize]) -> Vec<(usize, Vec<ConversationMessage>)> {
    counts.iter().map(|&count| {
        let messages: Vec<ConversationMessage> = (0..count)
            .map(|i| create_test_message(i))
            .collect();
        (count, messages)
    }).collect()
}

fn create_test_strings(lengths: &[usize]) -> Vec<(usize, String)> {
    lengths.iter().map(|&len| {
        let base = "This is test content that represents a typical message ";
        let repeats = (len / base.len()) + 1;
        let string = base.repeat(repeats);
        let string = string[..len.min(string.len())].to_string();
        (len, string)
    }).collect()
}

fn create_test_message(i: usize) -> ConversationMessage {
    ConversationMessage {
        application: Application::ClaudeCode,
        date: chrono::Utc::now(),
        project_hash: format!("project_{}", i % 100),
        conversation_hash: format!("conv_{}", i % 50),
        local_hash: Some(format!("local_hash_{}", i)),
        global_hash: format!("global_hash_{}", i),
        model: Some("claude-3-sonnet-20240229".to_string()),
        stats: splitrail_dashboard::types::Stats {
            input_tokens: 1000 + (i as u64 % 5000),
            output_tokens: 500 + (i as u64 % 2000),
            cost: 0.01 + (i as f64 * 0.001),
            tool_calls: (i % 10) as u32,
            ..Default::default()
        },
        role: if i % 2 == 0 { MessageRole::User } else { MessageRole::Assistant },
        content: Some(format!("Test message content {}", i)),
    }
}

fn parse_jsonl_file_realistic(file_path: &std::path::Path) -> Vec<serde_json::Value> {
    use std::io::{BufRead, BufReader};
    let file = File::open(file_path).unwrap();
    let reader = BufReader::new(file);
    
    reader.lines()
        .filter_map(|line| line.ok())
        .filter_map(|line| serde_json::from_str(&line).ok())
        .collect()
}

criterion_group!(
    benches,
    bench_real_bottlenecks,
    bench_file_system_ops,
    bench_memory_patterns
);
criterion_main!(benches);