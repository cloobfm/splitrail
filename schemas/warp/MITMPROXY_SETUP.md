# Warp mitmproxy Setup Guide

Complete guide to capturing Warp's GraphQL responses using mitmproxy.

## Overview

This setup intercepts HTTPS traffic between Warp and `app.warp.dev` to capture:
- **Credit usage data** (costs, remaining credits)
- **Model information** (which models are being used)
- **Token usage** (input/output tokens)
- **Feature availability** (model choices, credit multipliers)
- **All other usage metrics** shown in Warp's UI

## Prerequisites

- ✅ mitmproxy installed (already done)
- ✅ Interceptor script created
- ✅ Proxy control scripts created

## Setup Steps

### Step 1: Generate mitmproxy Certificate

First, start and stop the proxy to generate the SSL certificate:

```bash
cd ~/Projects/splitrail

# Start proxy in background
./scripts/start_warp_proxy_quiet.sh

# Wait 3 seconds for it to initialize
sleep 3

# Stop it (certificate is now generated)
./scripts/stop_warp_proxy.sh
```

This creates the certificate at `~/.mitmproxy/mitmproxy-ca-cert.pem`

### Step 2: Install Certificate to macOS Keychain

```bash
# Make the installation script executable
chmod +x ./scripts/install_mitmproxy_cert.sh

# Run it (will ask for sudo password)
./scripts/install_mitmproxy_cert.sh
```

This installs the mitmproxy certificate as a trusted root certificate.

**Alternative Manual Installation:**
1. Open **Keychain Access** app
2. Drag `~/.mitmproxy/mitmproxy-ca-cert.pem` to **System** keychain
3. Double-click the "mitmproxy" certificate
4. Expand **Trust** section
5. Set "When using this certificate" to **Always Trust**
6. Close and enter your password

### Step 3: Configure Warp to Use Proxy

Warp doesn't have built-in proxy settings, so we need to set system-wide proxy for the terminal session:

**Option A: Environment Variables (Per-Session)**

Add to your shell session before starting Warp:

```bash
export HTTP_PROXY=http://127.0.0.1:8080
export HTTPS_PROXY=http://127.0.0.1:8080
```

**Option B: System-Wide Proxy (Recommended)**

1. Open **System Settings** → **Network**
2. Select your active network (Wi-Fi or Ethernet)
3. Click **Details...**
4. Go to **Proxies** tab
5. Check **Web Proxy (HTTP)** and **Secure Web Proxy (HTTPS)**
6. For both, set:
   - Server: `127.0.0.1`
   - Port: `8080`
7. Click **OK**

**IMPORTANT**: Remember to turn off system proxy when you're done capturing!

### Step 4: Start the Proxy

**Interactive Mode** (see traffic in real-time):
```bash
./scripts/start_warp_proxy.sh
```

**Background Mode** (runs quietly):
```bash
./scripts/start_warp_proxy_quiet.sh

# View logs
tail -f ~/.mitmproxy/mitmdump.log

# Stop when done
./scripts/stop_warp_proxy.sh
```

### Step 5: Use Warp

With the proxy running and configured:

1. Open Warp terminal
2. Use the AI agent (ask questions, run commands, generate code)
3. The proxy will capture all GraphQL requests/responses

You'll see output like:
```
[20:30:45] Request: GetRequestLimitInfo
[20:30:45] Captured: GetRequestLimitInfo
[20:30:52] Request: GetFeatureModelChoices
[20:30:52] Captured: GetFeatureModelChoices
```

### Step 6: View Captured Data

Data is saved to `~/Projects/splitrail/schemas/warp/captured/`

**All GraphQL responses:**
```bash
cat ~/Projects/splitrail/schemas/warp/captured/graphql_responses_$(date +%Y%m%d).jsonl | jq .
```

**Usage data only (credits, models, limits):**
```bash
cat ~/Projects/splitrail/schemas/warp/captured/usage_data_$(date +%Y%m%d).jsonl | jq .
```

**Example: View credit information:**
```bash
cat ~/Projects/splitrail/schemas/warp/captured/usage_data_*.jsonl | \
    jq 'select(.type == "request_limit_info") | .data'
```

## What Gets Captured

### Credit & Usage Information

From `GetRequestLimitInfo` responses:
- `requestLimit` - Total requests allowed
- `requestsUsedSinceLastRefresh` - Current usage
- `nextRefreshTime` - When limits reset
- `requestCreditsGranted` - Bonus credits
- `requestCreditsRemaining` - Credits left
- `costCents` - Cost tracking

### Model Information

From `GetFeatureModelChoices` responses:
- Available models (GPT-5, Claude 4.5, etc.)
- Credit multipliers per model
- Quality/speed/cost ratings
- Vision support status
- Provider information

### Per-Request Metrics

While the proxy captures these GraphQL operations, the actual **per-request credit costs and token counts** may still be computed client-side and not transmitted in every response. The telemetry events we're already capturing have timing data.

## Stopping and Cleanup

### Stop the Proxy

**If running in interactive mode:** Press `Ctrl+C`

**If running in background:**
```bash
./scripts/stop_warp_proxy.sh
```

### Disable System Proxy

**Important**: Turn off system proxy when done!

1. **System Settings** → **Network** → **Details** → **Proxies**
2. Uncheck **Web Proxy** and **Secure Web Proxy**
3. Click **OK**

Or use command line:
```bash
# Disable for Wi-Fi
sudo networksetup -setwebproxystate Wi-Fi off
sudo networksetup -setsecurewebproxystate Wi-Fi off
```

### Remove Certificate (Optional)

If you want to remove the mitmproxy certificate:

1. Open **Keychain Access**
2. Select **System** keychain
3. Search for "mitmproxy"
4. Right-click → Delete

## Troubleshooting

### Warp Shows SSL Certificate Errors

- Make sure certificate is installed in **System** keychain (not Login)
- Verify it's set to "Always Trust"
- Restart Warp after installing certificate

### No Traffic Being Captured

- Verify proxy is running: `cat ~/.mitmproxy/mitmdump.pid`
- Check system proxy settings are correct (127.0.0.1:8080)
- Restart Warp to pick up proxy settings
- Check logs: `tail -f ~/.mitmproxy/mitmdump.log`

### Warp Can't Connect to Internet

- Make sure proxy is actually running
- Check firewall isn't blocking port 8080
- Try disabling/re-enabling proxy settings

### Other Apps Affected by Proxy

System-wide proxy affects all apps. To limit to Warp only:

1. Don't use system proxy
2. Use environment variables instead
3. Or run proxy only when actively capturing

## Privacy & Security Notes

- **Local only**: All captured data stays on your machine
- **No external transmission**: mitmproxy doesn't send data anywhere
- **Your credentials**: The proxy can see your Warp auth tokens (they're in the logs)
- **Keep logs private**: Don't share captured data - it contains your API tokens

## Integration with Splitrail

Once you've captured data, you can:

1. **Parse the captured files** in the Warp analyzer
2. **Extract usage metrics** for daily stats
3. **Calculate costs** from actual credit data
4. **Track model usage** from feature choices

We can update the Warp analyzer to read from `schemas/warp/captured/*.jsonl` in addition to the network log.

## Quick Reference

```bash
# One-time setup
./scripts/start_warp_proxy_quiet.sh && sleep 3 && ./scripts/stop_warp_proxy.sh
chmod +x ./scripts/install_mitmproxy_cert.sh
./scripts/install_mitmproxy_cert.sh

# Enable system proxy (System Settings → Network → Proxies)
# Set HTTP/HTTPS proxy to 127.0.0.1:8080

# Start capturing
./scripts/start_warp_proxy_quiet.sh

# Use Warp normally...

# View captured data
tail -f ~/Projects/splitrail/schemas/warp/captured/usage_data_$(date +%Y%m%d).jsonl

# Stop capturing
./scripts/stop_warp_proxy.sh

# Disable system proxy (System Settings → Network → Proxies)
```
