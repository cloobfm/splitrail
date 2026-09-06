# Gemini CLI Deprecation Notice

## Summary

As of **v2.0.0-dash-0.16 (2026-09-06)**, the Gemini CLI analyzer is no longer registered in the
Splitrail Dashboard. The code remains in `src/analyzers/gemini_cli.rs` but does not run.

## Why?

1. **Google retired Gemini CLI for individual accounts.** On 2026-06-18 Gemini CLI and the Gemini
   Code Assist IDE extensions stopped serving requests for free, Google AI Pro, and Google AI
   Ultra accounts. The replacement is Antigravity CLI, part of the Antigravity 2.0 platform
   announced at Google I/O in May 2026. Enterprise Code Assist licenses and raw API-key
   authentication are unaffected, and the open-source repository still ships releases, but as a
   consumer tool it is gone.
2. **No new data will arrive.** For an individual account the `~/.gemini/tmp/*/chats/*.json`
   session files stop growing at the shutdown date. On the reference machine the last session
   was 2026-03-20.
3. **It carried a parse failure (BZL-10)** on some historical sessions. With no future data,
   fixing a parser for a frozen corpus was not worth the maintenance.

## What Changed?

**`src/main.rs`:**
```rust
// Before:
registry.register(GeminiCliAnalyzer::new());

// After:
// registry.register(GeminiCliAnalyzer::new()); // Retired 2026-09
```

- The Gemini CLI tab disappears from the TUI.
- Historical Gemini CLI usage is no longer counted locally or uploaded.
- The `Gemini CLI sessions failed to parse` startup warning no longer appears.
- `Application::GeminiCli` and the Gemini model pricing entries are kept, so previously uploaded
  data and any re-enabled analyzer continue to work.

## Re-enabling

Uncomment the registration in `create_analyzer_registry` in `src/main.rs`. Nothing else is
required.

## What About Antigravity?

- **Antigravity IDE** stores conversations as opaque protobuf files under
  `~/.gemini/antigravity/conversations/*.pb` with no published schema. Not analyzable without
  reverse engineering.
- **Antigravity CLI** reportedly writes per-conversation JSONL transcripts
  (`transcript.jsonl`, `transcript_full.jsonl`) with step-level records for requests, reasoning,
  and tool calls. Whether token usage is recorded is unconfirmed. A new analyzer could target
  this if the tool sees real use; it is not a continuation of the Gemini CLI analyzer.
