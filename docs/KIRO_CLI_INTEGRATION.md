# Kiro CLI Integration in Splitrail Dashboard

## Overview

Kiro CLI has been successfully integrated into the Splitrail Dashboard, enabling real-time monitoring and analysis of Kiro CLI usage alongside other AI coding tools.

## Key Discovery

Our analysis revealed that **Kiro CLI and Amazon Q use identical database schemas and storage patterns**. This discovery enabled seamless integration by leveraging the existing Amazon Q analyzer infrastructure.

### Architectural Similarity

Both tools share:
- **Identical SQLite database schema** with 5 tables:
  - `auth_kv` - Authentication key-value store
  - `conversations` - Chat/conversation history  
  - `history` - Command history
  - `migrations` - Database version tracking
  - `state` - Application state

- **Same data storage locations**:
  - macOS: `~/Library/Application Support/<tool-name>/data.sqlite3`
  - Linux: `~/.config/<tool-name>/data.sqlite3` or `~/.local/share/<tool-name>/data.sqlite3`
  - Windows: `%APPDATA%/<tool-name>/data.sqlite3`

- **Identical conversation JSON structure** stored in the `conversations` table

### KIRO Takeover of Amazon Q

During integration testing, we discovered that **Kiro CLI has taken over Amazon Q**:

```bash
$ cat ~/.local/bin/q
#!/bin/sh
"/Users/bzl/.local/bin/kiro-cli" --show-legacy-warning "$@"
```

The `q` command (Amazon Q's CLI) now redirects to `kiro-cli` with a legacy warning, indicating Kiro CLI is the successor or replacement for Amazon Q.

## Implementation

### File Structure

```
src/analyzers/
├── amazon_q.rs         # Base analyzer with shared parsing logic
├── kiro_cli.rs         # Kiro CLI analyzer (reuses Amazon Q parser)
└── mod.rs              # Module exports
```

### Code Reuse Strategy

The Kiro CLI analyzer (`kiro_cli.rs`) reuses the Amazon Q conversation parser since they share identical JSON structures:

```rust
// In kiro_cli.rs
use crate::analyzers::amazon_q::parse_amazon_q_conversation;

// Parse conversations using the shared Amazon Q parser
let all_entries: Vec<ConversationMessage> = conversations
    .into_par_iter()
    .flat_map(|(project_path, conversation_json)| {
        match parse_amazon_q_conversation(&project_path, &conversation_json) {
            Ok(mut messages) => {
                // Override application type to Kiro CLI
                for msg in &mut messages {
                    msg.application = Application::KiroCli;
                }
                messages
            }
            Err(_e) => Vec::new()
        }
    })
    .collect();
```

### Application Type

Added `KiroCli` variant to the `Application` enum in `src/types.rs`:

```rust
pub enum Application {
    ClaudeCode,
    GeminiCli,
    QwenCode,
    CodexCli,
    Cline,
    RooCode,
    KiloCode,
    Copilot,
    AmazonQ,
    KiroCli,  // <-- New
    Warp,
}
```

### Analyzer Registration

Registered in `src/main.rs`:

```rust
fn create_analyzer_registry() -> AnalyzerRegistry {
    let mut registry = AnalyzerRegistry::new();
    
    // ... other analyzers
    registry.register(AmazonQAnalyzer::new());
    registry.register(KiroCliAnalyzer::new());  // <-- New
    
    registry
}
```

## Dashboard Features

Once integrated, Kiro CLI data appears in the Splitrail dashboard with:

- **Real-time usage tracking** - Token consumption, cost, and activity
- **Conversation monitoring** - Track all Kiro CLI conversations
- **Tool usage analysis** - Monitor file operations, shell commands, searches
- **Historical trends** - Daily statistics and usage patterns
- **Cost calculations** - Model-based pricing for token usage

## Data Tracked

The Kiro CLI analyzer tracks:

### Token & Cost Metrics
- Input tokens
- Output tokens
- Cache creation tokens
- Cache read tokens
- Total cost (model-based pricing)

### Tool Operations
- File operations (read, write, edit, delete)
- Terminal commands executed
- File searches and content searches
- Tool calls made

### Conversation Metadata
- Conversation IDs and hashes
- Project associations
- Model information
- Timestamps
- Message roles (user/assistant)

## Testing

### Unit Tests

The Kiro CLI analyzer includes tests in `src/analyzers/kiro_cli.rs`:

```rust
#[test]
fn test_kiro_cli_database_path() {
    if let Some(db_path) = KiroCliAnalyzer::get_database_path() {
        assert!(db_path.exists());
        assert_eq!(db_path.file_name().unwrap(), "data.sqlite3");
    }
}

#[test]
fn test_kiro_cli_uses_amazon_q_schema() {
    if let Some(db_path) = KiroCliAnalyzer::get_database_path() {
        let conn = rusqlite::Connection::open(db_path).unwrap();
        
        // Verify expected tables exist
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table'")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert!(tables.contains(&"conversations".to_string()));
        assert!(tables.contains(&"history".to_string()));
        assert!(tables.contains(&"state".to_string()));
        assert!(tables.contains(&"auth_kv".to_string()));
        assert!(tables.contains(&"migrations".to_string()));
    }
}
```

### Manual Testing

Run the dashboard to verify Kiro CLI detection:

```bash
cargo run --release
```

If Kiro CLI is installed and has data, you should see:
- "Kiro CLI" in the list of available analyzers
- Real-time statistics for Kiro CLI usage
- Conversation and token data in the dashboard

## Database Analysis Tool

A companion analysis tool was created to investigate and document the identical architecture:

```bash
./scripts/analyze_cli_wrappers.sh --compare
```

This tool:
- Detects both Amazon Q and Kiro CLI installations
- Compares database schemas (proves they're identical)
- Shows shell integration status
- Reveals the takeover relationship
- Provides removal instructions if needed

See `scripts/CLI_WRAPPER_ANALYZER.md` for full documentation.

## Future Enhancements

### Potential Improvements

1. **Unified Amazon Q/Kiro CLI View**
   - Since Kiro CLI replaced Amazon Q, consider merging their stats
   - Add a configuration option to treat them as the same tool

2. **Migration Detection**
   - Detect when Amazon Q data was migrated to Kiro CLI
   - Handle historical data spanning both tools

3. **Shell Integration Monitoring**
   - Track shell wrapper status
   - Alert when shell commands are being intercepted
   - Monitor performance impact

4. **Data Export**
   - Export Kiro CLI conversation history
   - Backup functionality for the SQLite database
   - Migration tools between different systems

## Security Considerations

### Data Privacy

Both Amazon Q and Kiro CLI store:
- Complete command history (unencrypted)
- Full conversation transcripts
- Project file paths and working directories
- Tool usage patterns

Users should be aware that:
- All terminal commands pass through Kiro CLI wrappers
- Command history is stored locally in SQLite
- Conversations may contain sensitive information

### Shell Integration

Kiro CLI installs shell wrappers (`~/.local/bin/bash`, `zsh`, etc.) that intercept every command. The analyzer respects this architecture but provides visibility into what's being tracked.

## Troubleshooting

### Kiro CLI Not Detected

If the dashboard doesn't show Kiro CLI:

1. **Verify installation**:
   ```bash
   ls -la ~/Library/Application\ Support/kiro-cli/
   ```

2. **Check database exists**:
   ```bash
   file ~/Library/Application\ Support/kiro-cli/data.sqlite3
   ```

3. **Verify database schema**:
   ```bash
   sqlite3 ~/Library/Application\ Support/kiro-cli/data.sqlite3 ".tables"
   ```
   Should show: `auth_kv conversations history migrations state`

### No Data Showing

If Kiro CLI is detected but no data appears:

1. **Check for conversations**:
   ```bash
   sqlite3 ~/Library/Application\ Support/kiro-cli/data.sqlite3 \
     "SELECT COUNT(*) FROM conversations"
   ```

2. **Verify JSON format**:
   ```bash
   sqlite3 ~/Library/Application\ Support/kiro-cli/data.sqlite3 \
     "SELECT value FROM conversations LIMIT 1" | jq .
   ```

3. **Review analyzer logs** in the dashboard for parsing errors

## Contributing

### Adding Support for Similar Tools

If you discover other CLI tools using the same architecture:

1. Verify database schema matches Amazon Q/Kiro CLI
2. Create a new analyzer in `src/analyzers/`
3. Reuse `parse_amazon_q_conversation()` 
4. Add application variant to `Application` enum
5. Register in `create_analyzer_registry()`
6. Update documentation

Example structure:
```rust
// src/analyzers/new_tool.rs
use crate::analyzers::amazon_q::parse_amazon_q_conversation;

pub struct NewToolAnalyzer;

impl Analyzer for NewToolAnalyzer {
    fn display_name(&self) -> &'static str {
        "New Tool"
    }
    
    // Implement remaining methods similar to kiro_cli.rs
}
```

## References

- **Amazon Q Analyzer**: `src/analyzers/amazon_q.rs`
- **Kiro CLI Analyzer**: `src/analyzers/kiro_cli.rs`
- **Database Analysis**: `scripts/analyze_cli_wrappers.sh`
- **Main Integration**: `src/main.rs`

## License

Same as Splitrail project (MIT License).
