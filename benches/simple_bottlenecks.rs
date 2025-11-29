use criterion::{black_box, criterion_group, criterion_main, Criterion};
use sha2::Digest;

// Test the ACTUAL operations we suspect are slow

fn bench_json_parsing(c: &mut Criterion) {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");
    
    // Create a realistic JSONL file
    {
        use std::io::Write;
        let mut file = std::fs::File::create(&file_path).unwrap();
        for i in 0..1000 {
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
    }
    
    c.bench_function("json_parsing_1000_lines", |b| {
        b.iter(|| {
            use std::io::{BufRead, BufReader};
            let file = std::fs::File::open(&file_path).unwrap();
            let reader = BufReader::new(file);
            
            let count: usize = reader.lines()
                .filter_map(|line| line.ok())
                .filter_map(|line| serde_json::from_str::<serde_json::Value>(&line).ok())
                .count();
            
            black_box(count)
        });
    });
}

fn bench_string_operations(c: &mut Criterion) {
    let test_string = "This is a test message content that simulates real UI formatting with numbers and tokens";
    
    // Test string formatting (common in UI)
    c.bench_function("string_formatting", |b| {
        b.iter(|| {
            for _ in 0..1000 {
                let _ = black_box(format!("{}: {} tokens", 
                    black_box(&test_string[..20]), 
                    black_box(1234567)
                ));
            }
        });
    });
    
    // Test string cloning (common in message processing)
    c.bench_function("string_cloning", |b| {
        b.iter(|| {
            for _ in 0..1000 {
                let _ = black_box(test_string.clone());
            }
        });
    });
}

fn bench_file_operations(c: &mut Criterion) {
    let temp_dir = tempfile::TempDir::new().unwrap();
    
    // Test file reading (I/O bottleneck)
    c.bench_function("file_read_1mb", |b| {
        // Create 1MB test file
        let file_path = temp_dir.path().join("test.txt");
        let content = "x".repeat(1024 * 1024);
        std::fs::write(&file_path, &content).unwrap();
        
        b.iter(|| {
            let _ = black_box(std::fs::read_to_string(&file_path).unwrap());
        });
    });
    
    // Test file metadata operations
    c.bench_function("file_metadata", |b| {
        b.iter(|| {
            let _ = black_box(std::fs::metadata("/tmp").unwrap());
            let _ = black_box(std::fs::metadata("/usr").unwrap());
            let _ = black_box(std::fs::metadata("/home").unwrap());
        });
    });
}

fn bench_hash_operations(c: &mut Criterion) {
    let test_strings = vec![
        "short string",
        "medium length string that represents a typical message",
        "very long string that represents a large message with lots of content and data that needs to be processed and hashed efficiently"
    ];
    
    c.bench_function("sha256_hashing", |b| {
        b.iter(|| {
            for s in &test_strings {
                let mut hasher = sha2::Sha256::new();
                hasher.update(s.as_bytes());
                let _ = black_box(hasher.finalize());
            }
        });
    });
}

criterion_group!(
    benches,
    bench_json_parsing,
    bench_string_operations,
    bench_file_operations,
    bench_hash_operations
);
criterion_main!(benches);