use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId, Throughput};
use splitrail_dashboard::utils::{hash_text, aggregate_by_date};
use splitrail_dashboard::types::{ConversationMessage, Stats, Application, MessageRole};
use chrono::Utc;
use std::time::Duration;

/// Performance regression tests to ensure optimizations don't break functionality
/// and maintain or improve performance characteristics.

const BASELINE_MESSAGE_COUNT: usize = 10000;
const BASELINE_TEXT_SIZE: usize = 1000;

fn bench_hash_performance_regression(c: &mut Criterion) {
    let mut group = c.benchmark_group("hash_performance_regression");
    group.measurement_time(Duration::from_secs(5));
    
    // Test hash performance with different text sizes
    for size in [100, 500, 1000, 5000, 10000].iter() {
        group.throughput(Throughput::Bytes(*size as u64));
        group.bench_with_input(
            BenchmarkId::new("hash_text", size),
            size,
            |b, &size| {
                let text = "x".repeat(size);
                b.iter(|| {
                    let _ = black_box(hash_text(black_box(&text)));
                });
            },
        );
    }
    
    group.finish();
}

fn bench_aggregation_performance_regression(c: &mut Criterion) {
    let mut group = c.benchmark_group("aggregation_performance_regression");
    group.measurement_time(Duration::from_secs(10));
    
    // Test aggregation with different message counts
    for msg_count in [1000, 5000, 10000, 25000].iter() {
        group.throughput(Throughput::Elements(*msg_count as u64));
        group.bench_with_input(
            BenchmarkId::new("aggregate_by_date", msg_count),
            msg_count,
            |b, &msg_count| {
                let messages = create_realistic_messages(msg_count);
                b.iter(|| {
                    let _ = black_box(aggregate_by_date(black_box(&messages)));
                });
            },
        );
    }
    
    group.finish();
}

fn bench_deduplication_performance_regression(c: &mut Criterion) {
    let mut group = c.benchmark_group("deduplication_performance_regression");
    group.measurement_time(Duration::from_secs(10));
    
    // Test current deduplication implementation
    for msg_count in [1000, 5000, 10000, 20000].iter() {
        group.throughput(Throughput::Elements(*msg_count as u64));
        group.bench_with_input(
            BenchmarkId::new("current_deduplication", msg_count),
            msg_count,
            |b, &msg_count| {
                let messages = create_messages_with_duplicates(msg_count);
                b.iter(|| {
                    let _ = black_box(deduplicate_messages_current(black_box(messages.clone())));
                });
            },
        );
    }
    
    group.finish();
}

fn bench_memory_allocation_patterns(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_allocation_patterns");
    
    // Test Vec allocation patterns
    group.bench_function("vec_growth_without_capacity", |b| {
        b.iter(|| {
            let mut vec: Vec<usize> = Vec::new();
            for i in 0..BASELINE_MESSAGE_COUNT {
                vec.push(black_box(i));
            }
            black_box(vec)
        });
    });
    
    group.bench_function("vec_growth_with_capacity", |b| {
        b.iter(|| {
            let mut vec: Vec<usize> = Vec::with_capacity(BASELINE_MESSAGE_COUNT);
            for i in 0..BASELINE_MESSAGE_COUNT {
                vec.push(black_box(i));
            }
            black_box(vec)
        });
    });
    
    // Test string operations
    let test_string = "x".repeat(BASELINE_TEXT_SIZE);
    
    group.bench_function("string_cloning", |b| {
        b.iter(|| {
            let _ = black_box(test_string.clone());
        });
    });
    
    group.finish();
}

fn bench_json_parsing_performance(c: &mut Criterion) {
    let mut group = c.benchmark_group("json_parsing_performance");
    
    // Create test JSON data
    let small_json = create_test_json_message(100);
    let medium_json = create_test_json_message(1000);
    let large_json = create_test_json_message(10000);
    
    group.bench_function("small_json_parse", |b| {
        b.iter(|| {
            let _: serde_json::Value = serde_json::from_str(black_box(&small_json)).unwrap();
        });
    });
    
    group.bench_function("medium_json_parse", |b| {
        b.iter(|| {
            let _: serde_json::Value = serde_json::from_str(black_box(&medium_json)).unwrap();
        });
    });
    
    group.bench_function("large_json_parse", |b| {
        b.iter(|| {
            let _: serde_json::Value = serde_json::from_str(black_box(&large_json)).unwrap();
        });
    });
    
    group.finish();
}

// Helper functions for creating test data

fn create_realistic_messages(count: usize) -> Vec<ConversationMessage> {
    let mut messages = Vec::with_capacity(count);
    let base_time = Utc::now();
    
    for i in 0..count {
        // Spread messages across different dates
        let days_offset = (i % 30) as i64;
        let date = base_time + chrono::Duration::days(days_offset);
        
        messages.push(ConversationMessage {
            application: Application::ClaudeCode,
            date,
            project_hash: format!("project_{}", i % 100),
            conversation_hash: format!("conv_{}", i % 50),
            local_hash: Some(format!("local_hash_{}", i)),
            global_hash: format!("global_hash_{}", i),
            model: Some(match i % 3 {
                0 => "claude-3-sonnet-20240229",
                1 => "claude-3-haiku-20240307",
                _ => "claude-3-opus-20240229",
            }.to_string()),
            stats: Stats {
                input_tokens: 1000 + (i as u64 % 10000),
                output_tokens: 500 + (i as u64 % 5000),
                cost: 0.01 + ((i as u64 % 10000) as f64 * 0.000001),
                tool_calls: (i % 20) as u32,
                terminal_commands: (i % 10) as u64,
                file_searches: (i % 5) as u64,
                files_read: (i % 8) as u64,
                lines_read: 100 + (i as u64 % 1000),
                ..Default::default()
            },
            role: if i % 2 == 0 { MessageRole::User } else { MessageRole::Assistant },
            content: Some(format!("Realistic test message content {} with varying length to simulate real data", i)),
        });
    }
    
    messages
}

fn create_messages_with_duplicates(count: usize) -> Vec<ConversationMessage> {
    let mut messages = Vec::with_capacity(count);
    let base_time = Utc::now();
    
    for i in 0..count {
        // Create duplicates by using same local_hash for multiple messages
        let duplicate_factor = i % 10; // Every 10th message shares a hash
        
        messages.push(ConversationMessage {
            application: Application::ClaudeCode,
            date: base_time + chrono::Duration::days((i % 30) as i64),
            project_hash: format!("project_{}", i % 50),
            conversation_hash: format!("conv_{}", i % 25),
            local_hash: Some(format!("duplicate_hash_{}", duplicate_factor)),
            global_hash: format!("global_hash_{}", i),
            model: Some("claude-3-sonnet-20240229".to_string()),
            stats: Stats::default(),
            role: MessageRole::User,
            content: Some(format!("Message {}", i)),
        });
    }
    
    messages
}

fn create_test_json_message(size: usize) -> String {
    serde_json::json!({
        "type": "message",
        "message": {
            "id": "test_message_id",
            "content": "x".repeat(size),
            "timestamp": "2024-01-01T00:00:00Z",
            "role": "user",
            "model": "claude-3-sonnet-20240229",
            "usage": {
                "input_tokens": 1000,
                "output_tokens": 500
            },
            "metadata": {
                "project": "test_project",
                "conversation": "test_conversation",
                "extra_data": "x".repeat(size / 10)
            }
        }
    }).to_string()
}

// Current deduplication implementation (for regression testing)
fn deduplicate_messages_current(messages: Vec<ConversationMessage>) -> Vec<ConversationMessage> {
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

criterion_group!(
    regression_tests,
    bench_hash_performance_regression,
    bench_aggregation_performance_regression,
    bench_deduplication_performance_regression,
    bench_memory_allocation_patterns,
    bench_json_parsing_performance
);
criterion_main!(regression_tests);