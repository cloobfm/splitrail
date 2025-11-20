# Amazon Q Deprecation Notice

## Summary

As of this update, **Amazon Q has been removed from the Splitrail Dashboard** and replaced by **Kiro CLI**.

## Why?

Our analysis revealed that:

1. **Kiro CLI has completely replaced Amazon Q**
   - The `q` command now redirects to `kiro-cli`
   - Amazon Q's application bundle is no longer present
   - Both tools use identical database schemas and storage

2. **Same underlying technology**
   - Identical SQLite database structure
   - Same conversation JSON format
   - Same data storage locations
   - Same model usage (Claude Sonnet 4/4.5)

3. **No functional difference**
   - Both read from the same data sources
   - Tracking both would show duplicate data
   - Kiro CLI is the active, maintained version

## What Changed?

### Dashboard Changes

**Removed:**
- Amazon Q analyzer registration
- Amazon Q from supported platforms list

**Added:**
- Kiro CLI as the primary analyzer
- Note that Kiro CLI "replaces Amazon Q"

### Code Changes

**`src/main.rs`:**
```rust
// Before:
registry.register(AmazonQAnalyzer::new());
registry.register(KiroCliAnalyzer::new());

// After:
// registry.register(AmazonQAnalyzer::new()); // Replaced by Kiro CLI
registry.register(KiroCliAnalyzer::new());
```

**Dashboard display:**
```
Before:                    After:
- Amazon Q                 - Kiro CLI (replaces Amazon Q)
- Kiro CLI
```

### What Remains?

The Amazon Q analyzer code (`src/analyzers/amazon_q.rs`) **remains in the codebase** because:
- Kiro CLI analyzer depends on it for parsing
- Both tools use identical data structures
- The parser is shared via `parse_amazon_q_conversation()`

This is efficient code reuse, not duplication.

## Migration Path

### If You Have Amazon Q Data

Your existing Amazon Q data will be automatically picked up by Kiro CLI because:
1. Kiro CLI reads from `~/Library/Application Support/kiro-cli/`
2. Amazon Q data is in `~/Library/Application Support/amazon-q/`
3. These are separate databases, both accessible

**If you want to consolidate:**
1. Copy Amazon Q database to Kiro CLI location:
   ```bash
   cp ~/Library/Application\ Support/amazon-q/data.sqlite3 \
      ~/Library/Application\ Support/kiro-cli/data-amazon-q-backup.sqlite3
   ```

2. Or keep them separate - Kiro CLI only reads its own database

### If You Still Use Amazon Q

If you still have Amazon Q installed and want to track it:
1. Uncomment the registration in `src/main.rs`:
   ```rust
   registry.register(AmazonQAnalyzer::new());
   ```
2. Rebuild: `cargo build --release`

However, note that:
- Amazon Q is deprecated
- The `q` command redirects to Kiro CLI
- You're likely using Kiro CLI even if you think you're using Amazon Q

## Data Continuity

**Your data is safe!**
- Historical Amazon Q data remains in `~/Library/Application Support/amazon-q/`
- Kiro CLI data is in `~/Library/Application Support/kiro-cli/`
- Both databases are preserved
- No data is deleted by this change

## Dashboard Impact

### Before
```
Splitrail Dashboard
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Available Analyzers:
  • Claude Code
  • Cline
  • Kilo Code
  • Gemini CLI
  • Qwen Code
  • Codex CLI
  • Amazon Q         ← 5 conversations
  • Kiro CLI         ← 6 conversations
```

### After
```
Splitrail Dashboard
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Available Analyzers:
  • Claude Code
  • Cline
  • Kilo Code
  • Gemini CLI
  • Qwen Code
  • Codex CLI
  • Kiro CLI (replaces Amazon Q)  ← 6 conversations
```

**Result:** Cleaner dashboard, no duplicate entries.

## Technical Details

### Architecture Unchanged

```
User → Terminal → Kiro CLI → Models (Claude Sonnet 4/4.5)
                      ↓
                 Database (~/Library/Application Support/kiro-cli/data.sqlite3)
                      ↓
           Splitrail Dashboard (via KiroCliAnalyzer)
```

### Code Structure

```
src/analyzers/
├── amazon_q.rs          # Parser (shared)
├── kiro_cli.rs          # Kiro CLI analyzer (uses amazon_q parser)
└── mod.rs               # Exports both (amazon_q for code reuse)

src/main.rs              # Only registers KiroCliAnalyzer
```

### Model Pricing

All models used by Kiro CLI have pricing configured:
- `claude-sonnet-4`: $3/$15 per 1M tokens
- `claude-sonnet-4.5`: $3/$15 per 1M tokens  
- `auto`: Maps to claude-sonnet-4 pricing

## FAQ

### Q: Will my Amazon Q history disappear?
**A:** No. The database remains at `~/Library/Application Support/amazon-q/`. It's just not tracked in the dashboard.

### Q: Can I still see Amazon Q data?
**A:** Yes, uncomment `registry.register(AmazonQAnalyzer::new());` in `src/main.rs` and rebuild.

### Q: What if I need both tracked separately?
**A:** Uncomment the Amazon Q registration. However, note that in practice, you're likely only using Kiro CLI.

### Q: Is Amazon Q code deleted?
**A:** No. The analyzer remains in `src/analyzers/amazon_q.rs` because Kiro CLI depends on it.

### Q: What about the CLI wrapper analysis tool?
**A:** The analysis tool (`scripts/analyze_cli_wrappers.sh`) still detects and analyzes both Amazon Q and Kiro CLI installations for debugging purposes.

## Verification

After rebuilding, verify the change:

```bash
# Check Amazon Q is not registered
grep "AmazonQAnalyzer::new()" src/main.rs
# Should show: // registry.register(AmazonQAnalyzer::new()); // Replaced by Kiro CLI

# Check Kiro CLI is registered
grep "KiroCliAnalyzer::new()" src/main.rs  
# Should show: registry.register(KiroCliAnalyzer::new());

# Run dashboard
cargo run --release
# Should only show "Kiro CLI (replaces Amazon Q)", not "Amazon Q"
```

## Conclusion

This change:
- ✅ Simplifies the dashboard
- ✅ Reflects reality (Kiro CLI replaced Amazon Q)
- ✅ Removes duplicate tracking
- ✅ Preserves all data
- ✅ Maintains code reuse efficiency
- ✅ Can be easily reverted if needed

Kiro CLI is now the single source of truth for this tool's usage tracking.
