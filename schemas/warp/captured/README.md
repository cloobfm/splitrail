# WARP Captured Data

This directory contains captured GraphQL responses from WARP's API during development and testing.

## Contents

When you run the proxy capture scripts, this directory will contain:

- `graphql_responses_YYYYMMDD.jsonl` - Full GraphQL API responses
- `usage_data_YYYYMMDD.jsonl` - Extracted usage metrics

## Privacy Note

⚠️ **These files contain personal usage data and are excluded from git** (see `.gitignore`).

The captured data includes:
- Conversation titles
- Credit usage
- Model choices
- Tool usage statistics
- Context window utilization

## How to Capture Data

```bash
# Start the proxy
./scripts/start_warp_proxy_quiet.sh

# Use WARP normally
# ...

# Stop the proxy
./scripts/stop_warp_proxy.sh

# View captured data
ls -lh schemas/warp/captured/
```

## Sample Data for Development

If you need sample data for testing the analyzer, you can:

1. Use the captured data from your own usage
2. Create anonymized test fixtures
3. Generate synthetic data based on the schema in `WARP_SCHEMA.md`

The data structure is documented in:
- `schemas/warp/WARP_SCHEMA.md` - Complete schema
- `schemas/warp/ANALYSIS_SUMMARY.md` - Example analysis
