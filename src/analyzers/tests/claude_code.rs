use crate::analyzers::claude_code::{
    ClaudeCodeAnalyzer, calculate_cost_from_tokens, dedup_messages, extract_project_id,
    parse_jsonl_file,
};
use crate::types::{Application, ConversationMessage, MessageRole, Stats};
use chrono::{TimeZone, Utc};
use simd_json::json;
use std::io::{BufReader, Cursor};
use std::path::Path;
use std::sync::LazyLock;

// Test data for full conversation parsing
static JSONL_DATA: LazyLock<String> = LazyLock::new(|| {
    r#"{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"D:\\splitrail","sessionId":"502be1cb-cf86-ecce-fe62-89ddec1e7563","version":"1.0.51","type":"user","message":{"role":"user","content":"What is this repo about?"},"uuid":"ba7d3ce9-c931-1a41-836d-a88d85c7aa83","timestamp":"2025-08-02T14:05:11.425Z"}
{"parentUuid":"ba7d3ce9-c931-1a41-836d-a88d85c7aa83","isSidechain":false,"userType":"external","cwd":"D:\\splitrail","sessionId":"502be1cb-cf86-ecce-fe62-89ddec1e7563","version":"1.0.51","message":{"id":"msg_19163d6657d79828b47fd7","type":"message","role":"assistant","model":"claude-sonnet-4-20250514","content":[{"type":"text","text":"Splitrail is a comprehensive agentic AI coding tool usage analyzer written in Rust."}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":4,"cache_creation_input_tokens":16027,"cache_read_input_tokens":0,"output_tokens":7,"service_tier":"standard"}},"requestId":"req_9d519281655d7bb03077","type":"assistant","uuid":"62b38f0c-18fa-78a3-635f-8b62138ca773","timestamp":"2025-08-02T14:05:17.096Z"}
{"parentUuid":"62b38f0c-18fa-78a3-635f-8b62138ca773","isSidechain":false,"userType":"external","cwd":"D:\\splitrail","sessionId":"502be1cb-cf86-ecce-fe62-89ddec1e7563","version":"1.0.51","message":{"id":"msg_4ed05b6f83dffea6d28e91","type":"message","role":"assistant","model":"claude-sonnet-4-20250514","content":[{"type":"tool_use","id":"toolu_12345","name":"Read","input":{"file_path":"test.rs"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":10,"cache_creation_input_tokens":200,"cache_read_input_tokens":50,"output_tokens":15,"service_tier":"standard"}},"requestId":"req_tool_use_test","type":"assistant","uuid":"tool-use-uuid","timestamp":"2025-08-02T14:05:26.780Z"}
{"parentUuid":"tool-use-uuid","isSidechain":false,"userType":"external","cwd":"D:\\splitrail","sessionId":"502be1cb-cf86-ecce-fe62-89ddec1e7563","version":"1.0.51","type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_12345","type":"tool_result","content":"File contents here"}]},"uuid":"tool-result-uuid","timestamp":"2025-08-02T14:05:30.000Z","toolUseResult":{"type":"text","file":{"filePath":"test.rs","content":"fn main() {}","numLines":1,"startLine":1,"totalLines":1}}}"#.to_string()
});

// Test data with tool operations for extract_tool_stats testing
static TOOL_OPERATIONS_DATA: LazyLock<String> = LazyLock::new(|| {
    r#"{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"D:\\splitrail","sessionId":"test-session","version":"1.0.51","message":{"id":"msg_edit_test","type":"message","role":"assistant","model":"claude-sonnet-4-20250514","content":[{"type":"tool_use","id":"toolu_edit1","name":"Edit","input":{"file_path":"test.rs","old_string":"old","new_string":"new"}},{"type":"tool_use","id":"toolu_bash1","name":"Bash","input":{"command":"ls -la"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":10,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":20,"service_tier":"standard"}},"requestId":"req_multi_tool","type":"assistant","uuid":"multi-tool-uuid","timestamp":"2025-08-02T15:00:00.000Z"}
{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"D:\\splitrail","sessionId":"test-session","version":"1.0.51","type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_todo_write","type":"tool_result","content":"Todo updated"}]},"uuid":"todo-result-uuid","timestamp":"2025-08-02T15:01:00.000Z","toolUseResult":{"oldTodos":[{"id":"1","title":"Task 1","status":"pending","priority":"high"}],"newTodos":[{"id":"1","title":"Task 1","status":"completed","priority":"high"},{"id":"2","title":"Task 2","status":"in_progress","priority":"medium"}]}}"#.to_string()
});

// Test data for duplicate messages - second message has higher token usage
static DUPLICATE_MESSAGES_DATA: LazyLock<String> = LazyLock::new(|| {
    r#"{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"D:\\splitrail","sessionId":"dup-session","version":"1.0.51","message":{"id":"msg_duplicate","type":"message","role":"assistant","model":"claude-sonnet-4-20250514","content":[{"type":"text","text":"First message"}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":10,"cache_creation_input_tokens":100,"cache_read_input_tokens":20,"output_tokens":5,"service_tier":"standard"}},"requestId":"req_dup_test","type":"assistant","uuid":"dup-uuid-1","timestamp":"2025-08-02T16:00:00.000Z"}
{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"D:\\splitrail","sessionId":"dup-session","version":"1.0.51","message":{"id":"msg_duplicate","type":"message","role":"assistant","model":"claude-sonnet-4-20250514","content":[{"type":"text","text":"Second message (duplicate)"}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":15,"cache_creation_input_tokens":150,"cache_read_input_tokens":30,"output_tokens":10,"service_tier":"standard"}},"requestId":"req_dup_test","type":"assistant","uuid":"dup-uuid-2","timestamp":"2025-08-02T16:00:01.000Z"}"#.to_string()
});

#[test]
fn test_parse_jsonl_file_basic() {
    let cursor = Cursor::new(JSONL_DATA.clone());
    let mut buf_reader = BufReader::new(cursor);
    let messages = parse_jsonl_file(Path::new("test.jsonl"), &mut buf_reader);

    // Four lines in the fixture, but the trailing tool_result-only user turn is API plumbing
    // and is dropped by the parser (see commit ca8b2a0), so three messages come back.
    assert_eq!(messages.len(), 3);

    // Check first message (user message, no usage or tool results -> User)
    assert_eq!(messages[0].role, MessageRole::User);
    assert_eq!(messages[0].application, Application::ClaudeCode);
    assert_eq!(messages[0].stats.input_tokens, 0);
    assert_eq!(messages[0].stats.output_tokens, 0);

    // Check second message (assistant with usage -> Assistant)
    assert_eq!(messages[1].role, MessageRole::Assistant);
    assert_eq!(messages[1].stats.input_tokens, 4);
    assert_eq!(messages[1].stats.cache_creation_tokens, 16027);
    assert_eq!(messages[1].stats.cache_read_tokens, 0);
    assert_eq!(messages[1].stats.output_tokens, 7);
    assert_eq!(
        messages[1].model,
        Some("claude-sonnet-4-20250514".to_string())
    );

    // Check third message (tool use with usage -> Assistant)
    assert_eq!(messages[2].role, MessageRole::Assistant);
    assert_eq!(messages[2].stats.tool_calls, 1);
    assert_eq!(messages[2].stats.files_read, 1);
    assert_eq!(messages[2].stats.input_tokens, 10);
    assert_eq!(messages[2].stats.output_tokens, 15);

    // The tool_result turn must not leak through under any role.
    assert!(messages.iter().all(|m| m.global_hash != "tool-result-uuid"));
}

#[test]
fn test_parse_jsonl_file_tool_operations() {
    let cursor = Cursor::new(TOOL_OPERATIONS_DATA.clone());
    let mut buf_reader = BufReader::new(cursor);
    let messages = parse_jsonl_file(Path::new("tools.jsonl"), &mut buf_reader);

    // The TodoWrite tool_result user turn is dropped by the parser (commit ca8b2a0), which
    // also means todos_completed / todos_in_progress are no longer derived from results.
    // Only the assistant's tool_use message survives.
    assert_eq!(messages.len(), 1);

    // Check multi-tool message
    let multi_tool_msg = &messages[0];
    assert_eq!(multi_tool_msg.role, MessageRole::Assistant);
    assert_eq!(multi_tool_msg.stats.tool_calls, 2);
    assert_eq!(multi_tool_msg.stats.files_edited, 1);
    assert_eq!(multi_tool_msg.stats.terminal_commands, 1);
}

#[test]
fn test_extract_and_hash_project_id() {
    let path1 = Path::new("/home/user/.claude/projects/proj123/conversation.jsonl");
    let path2 = Path::new("/home/user/.claude/projects/proj123/other.jsonl");
    let path3 = Path::new("/home/user/.claude/projects/proj456/conversation.jsonl");

    let hash1 = extract_project_id(path1);
    let hash2 = extract_project_id(path2);
    let hash3 = extract_project_id(path3);

    // Same project should have same hash
    assert_eq!(hash1, hash2);
    // Different projects should have different hashes
    assert_ne!(hash1, hash3);
    // Hashes should not be empty
    assert!(!hash1.is_empty());
    assert!(!hash3.is_empty());
}

#[test]
fn test_dedup_messages() {
    let cursor = Cursor::new(DUPLICATE_MESSAGES_DATA.clone());
    let mut buf_reader = BufReader::new(cursor);
    let messages = parse_jsonl_file(Path::new("duplicates.jsonl"), &mut buf_reader);

    // Should have 2 messages before deduplication
    assert_eq!(messages.len(), 2);

    let deduplicated = dedup_messages(messages.iter());

    // Deduplication now aggregates messages with the same local_hash
    assert_eq!(deduplicated.len(), 1);

    // Should sum the tokens from both entries: 10 + 15 = 25
    assert_eq!(deduplicated[0].stats.input_tokens, 25);

    // Test with manually created messages that should be deduplicated
    let mut test_messages = Vec::new();

    let base_msg = ConversationMessage {
        global_hash: "global1".to_string(),
        local_hash: Some("local1".to_string()),
        application: Application::ClaudeCode,
        model: Some("test-model".to_string()),
        date: Utc.timestamp_opt(1609459200, 0).unwrap(),
        project_hash: "project1".to_string(),
        conversation_hash: "conv1".to_string(),
        stats: Stats {
            input_tokens: 10,
            output_tokens: 5,
            cache_creation_tokens: 2,
            cache_read_tokens: 1,
            cached_tokens: 3,
            ..Default::default()
        },
        role: MessageRole::Assistant,
        content: Some("Test message content".to_string()),
    };

    let duplicate_msg = ConversationMessage {
        global_hash: "global2".to_string(),
        local_hash: Some("local1".to_string()), // Same local hash
        stats: Stats {
            input_tokens: 15, // Higher token counts than base_msg
            output_tokens: 8,
            cache_creation_tokens: 3,
            cache_read_tokens: 2,
            cached_tokens: 5,
            ..Default::default()
        },
        ..base_msg.clone()
    };

    test_messages.push(base_msg);
    test_messages.push(duplicate_msg.clone());

    let deduplicated_test = dedup_messages(test_messages.iter());
    // Aggregation logic merges messages with the same local_hash
    assert_eq!(deduplicated_test.len(), 1);
    // Should keep the first message's metadata but aggregate the tokens
    assert_eq!(deduplicated_test[0].global_hash, "global1");
    // Should sum the tokens from both entries: 10 + 15 = 25
    assert_eq!(deduplicated_test[0].stats.input_tokens, 25);
    // Should sum output tokens: 5 + 8 = 13
    assert_eq!(deduplicated_test[0].stats.output_tokens, 13);
    // Should sum cache tokens: 3 + 5 = 8
    assert_eq!(deduplicated_test[0].stats.cached_tokens, 8);
}

#[test]
fn test_calculate_cost_from_tokens() {
    use crate::analyzers::claude_code::Usage;

    let usage = Usage {
        input_tokens: 1000,
        output_tokens: 500,
        cache_creation_input_tokens: 200,
        cache_read_input_tokens: 100,
    };

    let cost = calculate_cost_from_tokens(&usage, "claude-sonnet-4-20250514");

    // Sonnet 4 pricing: $0.003/$0.015 per 1K input/output tokens
    // Cache creation: $0.00375 per 1K tokens, Cache read: $0.0003 per 1K tokens
    let expected_cost = (1000.0 * 0.003 / 1000.0) +  // Input tokens
        (500.0 * 0.015 / 1000.0) +   // Output tokens
        (200.0 * 0.00375 / 1000.0) + // Cache creation
        (100.0 * 0.0003 / 1000.0); // Cache read

    assert!(
        (cost - expected_cost).abs() < 0.0001,
        "Expected cost {expected_cost}, got {cost}"
    );
}

#[test]
fn test_extract_tool_stats_basic_tools() {
    use crate::analyzers::claude_code::{Content, ContentBlock, extract_tool_stats};

    let content = Content::Blocks(vec![
        ContentBlock::ToolUse {
            id: "tool1".to_string(),
            name: "Read".to_string(),
            input: json!({"file_path": "test.rs"}),
        },
        ContentBlock::ToolUse {
            id: "tool2".to_string(),
            name: "Edit".to_string(),
            input: json!({"file_path": "test.rs", "old_string": "old", "new_string": "new"}),
        },
        ContentBlock::ToolUse {
            id: "tool3".to_string(),
            name: "Bash".to_string(),
            input: json!({"command": "ls -la"}),
        },
    ]);

    let stats = extract_tool_stats(&content, &None);

    assert_eq!(stats.files_read, 1);
    assert_eq!(stats.files_edited, 1);
    assert_eq!(stats.terminal_commands, 1);
    assert_eq!(stats.files_added, 0);
    assert_eq!(stats.file_searches, 0);
}

#[test]
fn test_extract_tool_stats_all_tools() {
    use crate::analyzers::claude_code::{Content, ContentBlock, extract_tool_stats};

    let content = Content::Blocks(vec![
        ContentBlock::ToolUse {
            id: "tool1".to_string(),
            name: "Write".to_string(),
            input: json!({"file_path": "new.rs", "content": "fn main() {}"}),
        },
        ContentBlock::ToolUse {
            id: "tool2".to_string(),
            name: "MultiEdit".to_string(),
            input: json!({"file_path": "test.rs", "edits": []}),
        },
        ContentBlock::ToolUse {
            id: "tool3".to_string(),
            name: "Glob".to_string(),
            input: json!({"pattern": "*.rs"}),
        },
        ContentBlock::ToolUse {
            id: "tool4".to_string(),
            name: "Grep".to_string(),
            input: json!({"pattern": "fn main", "path": "."}),
        },
        ContentBlock::ToolUse {
            id: "tool5".to_string(),
            name: "TodoWrite".to_string(),
            input: json!({"todos": []}),
        },
        ContentBlock::ToolUse {
            id: "tool6".to_string(),
            name: "TodoRead".to_string(),
            input: json!({}),
        },
    ]);

    let stats = extract_tool_stats(&content, &None);

    assert_eq!(stats.files_added, 1);
    assert_eq!(stats.files_edited, 1);
    assert_eq!(stats.file_searches, 1);
    assert_eq!(stats.file_content_searches, 1);
    assert_eq!(stats.todo_writes, 1);
    assert_eq!(stats.todo_reads, 1);
}

#[test]
fn test_extract_tool_stats_with_todo_result() {
    use crate::analyzers::claude_code::{Content, extract_tool_stats};

    let tool_result = json!({
        "oldTodos": [
            {"id": "1", "title": "Task 1", "status": "pending", "priority": "high"},
            {"id": "2", "title": "Task 2", "status": "in_progress", "priority": "medium"},
            {"id": "3", "title": "Task 3", "status": "completed", "priority": "low"}
        ],
        "newTodos": [
            {"id": "1", "title": "Task 1", "status": "completed", "priority": "high"},
            {"id": "2", "title": "Task 2", "status": "in_progress", "priority": "medium"},
            {"id": "3", "title": "Task 3", "status": "completed", "priority": "low"},
            {"id": "4", "title": "Task 4", "status": "pending", "priority": "high"},
            {"id": "5", "title": "Task 5", "status": "in_progress", "priority": "medium"}
        ]
    });

    let content = Content::String(serde_bytes::ByteBuf::new());
    let stats = extract_tool_stats(&content, &Some(tool_result));

    // 2 new todos created (4 and 5)
    assert_eq!(stats.todos_created, 2);
    // 1 todo completed (task 1: pending -> completed)
    assert_eq!(stats.todos_completed, 1);
    // 1 todo moved to in_progress (task 5)
    assert_eq!(stats.todos_in_progress, 1);
}

#[test]
fn test_extract_tool_stats_text_content() {
    use crate::analyzers::claude_code::{Content, extract_tool_stats};

    let content = Content::String(serde_bytes::ByteBuf::new());
    let stats = extract_tool_stats(&content, &None);

    // Should be all zeros for text content
    assert_eq!(stats.files_read, 0);
    assert_eq!(stats.files_edited, 0);
    assert_eq!(stats.files_added, 0);
    assert_eq!(stats.terminal_commands, 0);
    assert_eq!(stats.file_searches, 0);
    assert_eq!(stats.file_content_searches, 0);
    assert_eq!(stats.todo_writes, 0);
    assert_eq!(stats.todo_reads, 0);
}

#[test]
fn test_extract_tool_stats_unknown_tools() {
    use crate::analyzers::claude_code::{Content, ContentBlock, extract_tool_stats};

    let content = Content::Blocks(vec![
        ContentBlock::ToolUse {
            id: "tool1".to_string(),
            name: "UnknownTool".to_string(),
            input: json!({"param": "value"}),
        },
        ContentBlock::ToolUse {
            id: "tool2".to_string(),
            name: "AnotherUnknownTool".to_string(),
            input: json!({}),
        },
    ]);

    let stats = extract_tool_stats(&content, &None);

    // Unknown tools should not increment any counters
    assert_eq!(stats.files_read, 0);
    assert_eq!(stats.files_edited, 0);
    assert_eq!(stats.files_added, 0);
    assert_eq!(stats.terminal_commands, 0);
    assert_eq!(stats.file_searches, 0);
    assert_eq!(stats.file_content_searches, 0);
    assert_eq!(stats.todo_writes, 0);
    assert_eq!(stats.todo_reads, 0);
}

#[test]
fn test_role_determination_logic() {
    // Role comes from message.role (commit 364c1a7). Tool-result-only user turns are dropped
    // (commit ca8b2a0). Usage on a message does not change its role.
    static ROLE_TEST_DATA: LazyLock<String> = LazyLock::new(|| {
        r#"{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"/test","sessionId":"role-test","version":"1.0.51","message":{"id":"msg_assistant","type":"message","role":"assistant","model":"claude-sonnet-4-20250514","content":[{"type":"text","text":"Assistant response"}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":10,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":5,"service_tier":"standard"}},"requestId":"req_usage_test","type":"assistant","uuid":"assistant-uuid","timestamp":"2025-08-02T17:00:00.000Z"}
{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"/test","sessionId":"role-test","version":"1.0.51","type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_123","type":"tool_result","content":"Tool result"}]},"uuid":"tool-result-uuid","timestamp":"2025-08-02T17:01:00.000Z","toolUseResult":{"type":"text","content":"Result content"}}
{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"/test","sessionId":"role-test","version":"1.0.51","type":"user","message":{"role":"user","content":"Plain user message"},"uuid":"plain-user-uuid","timestamp":"2025-08-02T17:02:00.000Z"}"#.to_string()
    });

    let cursor = Cursor::new(ROLE_TEST_DATA.clone());
    let mut buf_reader = BufReader::new(cursor);
    let messages = parse_jsonl_file(Path::new("role_test.jsonl"), &mut buf_reader);

    // Assistant + plain user. The tool_result turn is dropped.
    assert_eq!(messages.len(), 2);

    assert_eq!(messages[0].role, MessageRole::Assistant);
    assert_eq!(messages[0].stats.input_tokens, 10);
    assert_eq!(messages[0].stats.output_tokens, 5);

    assert_eq!(messages[1].role, MessageRole::User);
    assert_eq!(messages[1].stats.input_tokens, 0);
    assert_eq!(messages[1].stats.output_tokens, 0);
    assert_eq!(messages[1].content.as_deref(), Some("Plain user message"));
}

#[test]
fn test_unknown_entry_types_do_not_warn() {
    // Claude Code writes many housekeeping record types (attachment, cost-state, last-prompt, ...)
    // that carry no token usage. They must be skipped silently, not reported as invalid entries.
    let jsonl = concat!(
        r#"{"type":"last-prompt","leafUuid":"abc","sessionId":"s1"}"#, "\n",
        r#"{"type":"attachment","attachment":{"type":"file"},"cwd":"/tmp","sessionId":"s1","timestamp":"2026-09-05T00:00:00.000Z","uuid":"u1","parentUuid":null,"isSidechain":false,"userType":"external","version":"2.0.0","entrypoint":"cli","gitBranch":"main"}"#, "\n",
        r#"{"type":"cost-state","sessionId":"s1","totalCostUSD":1.5,"modelUsage":{}}"#, "\n",
        r#"{"type":"some-future-record","sessionId":"s1"}"#, "\n",
        // ToolSearch results: tool_result whose content is a list of `tool_reference` blocks.
        r#"{"parentUuid":"u2","isSidechain":false,"userType":"external","cwd":"/tmp","sessionId":"s1","version":"2.0.0","type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":[{"type":"tool_reference","tool_name":"WebSearch"}]}]},"uuid":"u3","timestamp":"2026-09-05T00:00:02.000Z","toolUseResult":{"matches":["WebSearch"],"query":"select:WebSearch"}}"#, "\n",
        // Assistant message with an unmodeled `fallback` block: its usage must still be counted.
        r#"{"parentUuid":"u3","isSidechain":false,"userType":"external","cwd":"/tmp","sessionId":"s1","version":"2.0.0","type":"assistant","message":{"id":"msg_2","role":"assistant","model":"claude-opus-5","content":[{"type":"fallback","model":"claude-opus-4-8"},{"type":"text","text":"ok"}],"usage":{"input_tokens":7,"output_tokens":3}},"uuid":"u4","timestamp":"2026-09-05T00:00:03.000Z","requestId":"req_2"}"#, "\n",
        r#"{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"/tmp","sessionId":"s1","version":"2.0.0","type":"assistant","message":{"id":"msg_1","role":"assistant","model":"claude-opus-5","content":[{"type":"text","text":"hi"}],"usage":{"input_tokens":10,"output_tokens":5}},"uuid":"u2","timestamp":"2026-09-05T00:00:01.000Z","requestId":"req_1"}"#, "\n",
    );
    let path = Path::new("/unknown-types-test/s1.jsonl");
    let mut reader = BufReader::new(Cursor::new(jsonl));
    let messages = parse_jsonl_file(path, &mut reader);

    // Tool-result-only user turns are intentionally dropped by the parser, so only the two
    // assistant messages come back. The tool_reference line is covered by the warning check.
    assert_eq!(messages.len(), 2, "both assistant messages should parse");
    assert!(
        messages.iter().any(|m| m.stats.input_tokens == 7),
        "usage survives an unmodeled block"
    );
    let noisy: Vec<String> = crate::utils::get_warnings()
        .into_iter()
        .filter(|w| w.contains("unknown-types-test"))
        .collect();
    assert!(noisy.is_empty(), "unknown entry types should not warn, got: {noisy:?}");
}

#[tokio::test]
async fn test_analyzer_reloads_incrementally() {
    use crate::analyzer::DataSource;
    use std::io::Write;

    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("-Users-test-proj");
    std::fs::create_dir_all(&project_dir).unwrap();
    let file = project_dir.join("session-1.jsonl");
    // Fixture lines end without a newline; a live file always has one.
    std::fs::write(&file, format!("{}\n", JSONL_DATA.as_str())).unwrap();
    let sources = vec![DataSource { path: file.clone() }];

    let analyzer = ClaudeCodeAnalyzer::new();

    let (first, report) = analyzer.parse_conversations_with_report(&sources);
    assert_eq!(first.len(), 3);
    assert_eq!(report.reparsed, 1);
    // Whatever label the file's first entry established must apply to later appends too.
    let project_hash = first[0].project_hash.clone();
    assert!(first.iter().all(|m| m.project_hash == project_hash));

    // Nothing changed: no bytes parsed, same result.
    let (second, report) = analyzer.parse_conversations_with_report(&sources);
    assert_eq!(second.len(), 3);
    assert_eq!(report.unchanged, 1);
    assert_eq!(report.bytes_parsed, 0);

    // Append one assistant turn; only it should be parsed, and it must inherit the project label
    // even though the entry that defined the label is in the already-parsed prefix.
    let appended = r#"{"parentUuid":"x","isSidechain":false,"userType":"external","cwd":"D:\\splitrail","sessionId":"502be1cb-cf86-ecce-fe62-89ddec1e7563","version":"1.0.51","type":"assistant","message":{"id":"msg_new","type":"message","role":"assistant","model":"claude-opus-5","content":[{"type":"text","text":"later"}],"usage":{"input_tokens":3,"output_tokens":4}},"requestId":"req_new","uuid":"new-uuid","timestamp":"2025-08-02T15:00:00.000Z"}"#;
    {
        let mut f = std::fs::OpenOptions::new().append(true).open(&file).unwrap();
        writeln!(f, "{appended}").unwrap();
    }
    let (third, report) = analyzer.parse_conversations_with_report(&sources);
    assert_eq!(third.len(), 4);
    assert_eq!(report.appended, 1);
    assert_eq!(report.bytes_parsed, appended.len() as u64 + 1);
    let new_msg = third.iter().find(|m| m.model.as_deref() == Some("claude-opus-5")).unwrap();
    assert_eq!(new_msg.project_hash, project_hash);
    assert_eq!(new_msg.stats.input_tokens, 3);
}
