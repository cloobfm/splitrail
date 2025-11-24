# WARP Sample Log Files

This directory contains sample WARP log files for development and testing.

## Contents

- `block.json` - Sample WARP block structure (committed)
- `warp_daily_*.log` - Daily WARP logs (gitignored - personal data)
- `warp_network_snapshot_*.log` - Network activity snapshots (gitignored - personal data)
- `.warp_offset` - Log offset tracking file (gitignored)

## Privacy Note

⚠️ **Log files contain personal command history and are excluded from git** (see `.gitignore`).

The log files may include:
- Shell commands you've executed
- Working directory paths
- Command timestamps
- AI suggestion requests
- Terminal interactions

## Log File Locations

WARP stores logs in:
- macOS: `~/Library/Application Support/Warp/warp_network.log`
- Alternative: `~/Library/Application Support/dev.warp.Warp-Stable/warp_network.log`

## Using Sample Data

To test the WARP analyzer with your own data:

```bash
# Copy current WARP log
cp ~/Library/Application\ Support/Warp/warp_network.log schemas/warp/samples/

# Run Splitrail analyzer
cargo run
```

## Data Format

Sample WARP log structure is documented in `block.json` (the only file committed to git).

For complete documentation, see:
- `schemas/warp/WARP_SCHEMA.md` - API schema
- `CLAUDE.md` - WARP integration section
