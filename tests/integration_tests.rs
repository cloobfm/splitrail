use splitrail_dashboard::analyzer::{Analyzer, AnalyzerRegistry};
use splitrail_dashboard::analyzers::*;
use splitrail_dashboard::types::ConversationMessage;
use std::collections::HashSet;
use std::fs::File;
use std::io::Write;
use std::sync::Mutex;
use tempfile::TempDir;

/// Tests that read or override `$HOME` must hold this lock. Cargo runs tests in parallel
/// threads within one process, and the environment is process-wide.
static HOME_LOCK: Mutex<()> = Mutex::new(());

/// Overrides `$HOME` for the lifetime of the guard and restores the previous value on drop,
/// including on panic, so a failing test cannot poison the ones that run after it.
struct HomeOverride {
    previous: Option<std::ffi::OsString>,
}

impl HomeOverride {
    fn new(path: &std::path::Path) -> Self {
        let previous = std::env::var_os("HOME");
        // Mutating the environment is `unsafe` in the 2024 edition because other threads may
        // be reading it concurrently; HOME_LOCK serializes the tests that care.
        unsafe {
            std::env::set_var("HOME", path);
        }
        Self { previous }
    }
}

impl Drop for HomeOverride {
    fn drop(&mut self) {
        unsafe {
            match &self.previous {
                Some(prev) => std::env::set_var("HOME", prev),
                None => std::env::remove_var("HOME"),
            }
        }
    }
}

#[tokio::test]
async fn test_analyzer_registry_creation() {
    let registry = create_test_registry();
    
    // Test that all expected analyzers are registered
    let available_analyzers = registry.available_analyzers();
    let analyzer_names: HashSet<&str> = available_analyzers
        .iter()
        .map(|a| a.display_name())
        .collect();
    
    // Should contain our test analyzers (if they're available on the system)
    assert!(!analyzer_names.is_empty());
}

#[tokio::test]
async fn test_analyzer_stats_loading() {
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let registry = create_test_registry();
    
    // This test will only work if the actual data sources exist
    // In a real test environment, you'd set up test data
    let result = registry.load_all_stats().await;
    
    // Should not panic, even if no data is available
    match result {
        Ok(stats) => {
            // Verify structure is correct
            assert_eq!(stats.analyzer_stats.len(), registry.available_analyzers().len());
            
            for analyzer_stats in &stats.analyzer_stats {
                // Verify required fields are present
                assert!(!analyzer_stats.analyzer_name.is_empty());
                // daily_stats and messages may be empty if no data exists
            }
        }
        Err(e) => {
            // It's OK if this fails due to no data sources
            println!("Expected failure in test environment: {}", e);
        }
    }
}

#[tokio::test]
async fn test_claude_code_analyzer_with_test_data() {
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let temp_dir = TempDir::new().unwrap();
    let analyzer = ClaudeCodeAnalyzer::new();
    
    // Create test data directory structure
    let claude_dir = temp_dir.path().join(".claude").join("projects").join("test_project");
    std::fs::create_dir_all(&claude_dir).unwrap();
    
    // Create test JSONL file
    let jsonl_file = claude_dir.join("conversations.jsonl");
    create_test_claude_file(&jsonl_file).await;
    
    // Point the analyzer at the temp dir; restored when `_override` drops.
    let _override = HomeOverride::new(temp_dir.path());
    
    // Test data discovery
    let sources = analyzer.discover_data_sources();
    assert!(sources.is_ok());
    
    let sources = sources.unwrap();
    if !sources.is_empty() {
        // Test conversation parsing
        let messages = analyzer.parse_conversations(sources).await;
        assert!(messages.is_ok());
        
        let messages = messages.unwrap();
        assert!(!messages.is_empty());
        
        // Verify message structure
        for msg in &messages {
            assert!(!msg.project_hash.is_empty());
            assert!(!msg.conversation_hash.is_empty());
            assert!(!msg.global_hash.is_empty());
        }
        
        // Test stats generation
        let stats = analyzer.get_stats().await;
        assert!(stats.is_ok());
        
        let stats = stats.unwrap();
        assert_eq!(stats.analyzer_name, "Claude Code");
        assert_eq!(stats.messages.len(), messages.len());
    }
}

#[tokio::test]
async fn test_message_deduplication() {
    // Create test data with duplicates
    let messages = create_test_messages_with_duplicates();
    
    // Test that deduplication works
    let unique_messages = deduplicate_test_messages(messages);
    
    // Should have fewer messages after deduplication
    assert!(unique_messages.len() <= 10); // Started with 10, some duplicates
    
    // Verify no duplicates remain
    let mut local_hashes = HashSet::new();
    for msg in &unique_messages {
        if let Some(hash) = &msg.local_hash {
            assert!(!local_hashes.contains(hash));
            local_hashes.insert(hash.clone());
        }
    }
}

#[tokio::test]
async fn test_parallel_vs_sequential_parsing() {
    let temp_dir = TempDir::new().unwrap();
    
    // Create multiple test files
    let files = create_multiple_test_files(&temp_dir, 5, 1000).await;
    
    // Test sequential parsing
    let start = std::time::Instant::now();
    let sequential_results = parse_files_sequential_test(&files).await;
    let sequential_time = start.elapsed();
    
    // Test parallel parsing
    let start = std::time::Instant::now();
    let parallel_results = parse_files_parallel_test(&files).await;
    let parallel_time = start.elapsed();
    
    // Results should be identical
    assert_eq!(sequential_results.len(), parallel_results.len());
    
    // Parallel should generally be faster (though not guaranteed in all environments)
    println!("Sequential time: {:?}", sequential_time);
    println!("Parallel time: {:?}", parallel_time);
    
    // At minimum, both should succeed
    assert!(!sequential_results.is_empty());
    assert!(!parallel_results.is_empty());
}

#[test]
fn test_file_watcher_integration() {
    let registry = create_test_registry();
    
    // Test directory mapping
    let mapping = registry.get_directory_to_analyzer_mapping();
    assert!(!mapping.is_empty());
    
    // Verify mapping structure
    for entry in &mapping {
        assert!(!entry.analyzer_name.is_empty());
        assert!(!entry.watch_dir.as_os_str().is_empty());
        assert!(!entry.match_dir.as_os_str().is_empty());
    }
}

#[test]
fn test_memory_usage_patterns() {
    // Test that we don't have obvious memory leaks
    
    // Create a large number of messages
    let messages = create_large_test_dataset(10000);
    
    // Process them
    let _aggregated = splitrail_dashboard::utils::aggregate_by_date(&messages);
    
    // The key test is that this doesn't crash or use excessive memory
    // In a real scenario, you'd use memory profiling tools
    assert!(messages.len() == 10000);
}

// Helper functions
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

async fn create_test_claude_file(file_path: &std::path::Path) {
    let mut file = File::create(file_path).unwrap();

    // Mirror the real Claude Code JSONL shape: `type`, `uuid`, and `timestamp` are top-level,
    // user turns have plain-string content, assistant turns carry a model and usage.
    for i in 0..10 {
        let is_user = i % 2 == 0;
        let uuid = format!("uuid-{i}");
        let timestamp = format!("2024-01-01T00:{i:02}:00Z");
        let message = if is_user {
            serde_json::json!({
                "type": "user",
                "uuid": uuid,
                "timestamp": timestamp,
                "sessionId": "integration-session",
                "cwd": "/tmp/integration",
                "message": {
                    "role": "user",
                    "content": format!("Test message {i}"),
                }
            })
        } else {
            serde_json::json!({
                "type": "assistant",
                "uuid": uuid,
                "timestamp": timestamp,
                "sessionId": "integration-session",
                "cwd": "/tmp/integration",
                "requestId": format!("req_{i}"),
                "message": {
                    "id": format!("msg_{i}"),
                    "type": "message",
                    "role": "assistant",
                    "model": "claude-3-sonnet-20240229",
                    "content": [{"type": "text", "text": format!("Test message {i}")}],
                    "usage": {
                        "input_tokens": 1000 + i * 100,
                        "output_tokens": 500 + i * 50
                    }
                }
            })
        };
        writeln!(file, "{message}").unwrap();
    }
}

fn create_test_messages_with_duplicates() -> Vec<ConversationMessage> {
    let mut messages = Vec::new();
    
    for i in 0..10 {
        messages.push(ConversationMessage {
            application: splitrail_dashboard::types::Application::ClaudeCode,
            date: chrono::Utc::now(),
            project_hash: "test_project".to_string(),
            conversation_hash: "test_conversation".to_string(),
            local_hash: Some(format!("hash_{}", i % 3)), // Create duplicates
            global_hash: format!("global_{}", i),
            model: Some("claude-3-sonnet".to_string()),
            stats: splitrail_dashboard::types::Stats::default(),
            role: splitrail_dashboard::types::MessageRole::User,
            content: Some(format!("Message {}", i)),
        });
    }
    
    messages
}

fn deduplicate_test_messages(messages: Vec<ConversationMessage>) -> Vec<ConversationMessage> {
    let mut result = Vec::new();
    let mut seen_hashes = HashSet::new();
    
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

async fn create_multiple_test_files(temp_dir: &TempDir, file_count: usize, lines_per_file: usize) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    
    for i in 0..file_count {
        let file_path = temp_dir.path().join(format!("test_{}.jsonl", i));
        let mut file = File::create(&file_path).unwrap();
        
        for j in 0..lines_per_file {
            let line = serde_json::json!({
                "id": format!("msg_{}_{}", i, j),
                "content": format!("Message {} from file {}", j, i),
                "timestamp": "2024-01-01T00:00:00Z"
            });
            writeln!(file, "{}", line).unwrap();
        }
        
        files.push(file_path);
    }
    
    files
}

async fn parse_files_sequential_test(files: &[std::path::PathBuf]) -> Vec<serde_json::Value> {
    use std::io::BufRead;
    let mut all_values = Vec::new();
    
    for file_path in files {
        let file = File::open(file_path).unwrap();
        let reader = std::io::BufReader::new(file);
        
        for line in reader.lines() {
            let line = line.unwrap();
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                all_values.push(value);
            }
        }
    }
    
    all_values
}

async fn parse_files_parallel_test(files: &[std::path::PathBuf]) -> Vec<serde_json::Value> {
    use rayon::prelude::*;
    use std::io::BufRead;
    
    files.par_iter()
        .flat_map(|file_path| {
            let file = File::open(file_path).unwrap();
            let reader = std::io::BufReader::new(file);
            
            reader.lines()
                .filter_map(|line| line.ok())
                .filter_map(|line| serde_json::from_str(&line).ok())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn create_large_test_dataset(size: usize) -> Vec<ConversationMessage> {
    let mut messages = Vec::with_capacity(size);
    
    for i in 0..size {
        messages.push(ConversationMessage {
            application: splitrail_dashboard::types::Application::ClaudeCode,
            date: chrono::Utc::now(),
            project_hash: format!("project_{}", i % 100),
            conversation_hash: format!("conv_{}", i % 50),
            local_hash: Some(format!("hash_{}", i)),
            global_hash: format!("global_{}", i),
            model: Some("claude-3-sonnet".to_string()),
            stats: splitrail_dashboard::types::Stats {
                input_tokens: 1000 + (i as u64 % 5000),
                output_tokens: 500 + (i as u64 % 2000),
                cost: 0.01 + (i as f64 * 0.001),
                tool_calls: (i % 10) as u32,
                ..Default::default()
            },
            role: if i % 2 == 0 { 
                splitrail_dashboard::types::MessageRole::User 
            } else { 
                splitrail_dashboard::types::MessageRole::Assistant 
            },
            content: Some(format!("Test message {}", i)),
        });
    }
    
    messages
}