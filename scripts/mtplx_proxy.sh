#!/bin/sh
# Put splitrail's usage proxy in front of an mtplx port (BZL-238), so the MTPLX column gets the
# transcript, the calling project and measured power for requests Pi does not already record.
#
#   scripts/mtplx_proxy.sh install 8000   # proxy on :8000, mtplx moves to :18000
#   scripts/mtplx_proxy.sh uninstall 8000
#
# Clients (Pi, OpenCode, the web UI) keep using :8000. Start mtplx itself on the upstream port
# afterwards, e.g. `mtplx start pi --port 18000`; until then the proxy answers 502.
set -eu
action=${1:-}; port=${2:-}
[ -n "$action" ] && [ -n "$port" ] || { echo "usage: $0 install|uninstall <port>" >&2; exit 2; }
upstream=$((port + 10000))
label="com.splitrail.mtplx-proxy-$port"
plist="$HOME/Library/LaunchAgents/$label.plist"
here=$(cd "$(dirname "$0")" && pwd)
case "$action" in
install)
    mkdir -p "$HOME/.config/splitrail/mtplx" "$HOME/Library/LaunchAgents"
    cat > "$plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>$label</string>
    <key>ProgramArguments</key>
    <array><string>/usr/bin/python3</string><string>$here/ollama_usage_proxy.py</string></array>
    <key>EnvironmentVariables</key>
    <dict>
        <key>OLLAMA_PROXY_HOST</key><string>127.0.0.1</string>
        <key>OLLAMA_PROXY_PORT</key><string>$port</string>
        <key>OLLAMA_UPSTREAM</key><string>http://127.0.0.1:$upstream</string>
        <key>OLLAMA_PROXY_USAGE_FILE</key><string>$HOME/.config/splitrail/mtplx/usage-$port.jsonl</string>
    </dict>
    <key>RunAtLoad</key><true/>
    <key>KeepAlive</key><true/>
    <key>ThrottleInterval</key><integer>30</integer>
    <key>StandardOutPath</key><string>$HOME/.splitrail.log</string>
    <key>StandardErrorPath</key><string>$HOME/.splitrail.log</string>
</dict>
</plist>
PLIST
    launchctl bootout "gui/$(id -u)/$label" 2>/dev/null || true
    launchctl bootstrap "gui/$(id -u)" "$plist"
    echo "proxy on :$port -> mtplx on :$upstream. Now start mtplx there: mtplx start pi --port $upstream"
    ;;
uninstall)
    launchctl bootout "gui/$(id -u)/$label" 2>/dev/null || true
    rm -f "$plist"
    echo "proxy on :$port removed; start mtplx on :$port again."
    ;;
*) echo "usage: $0 install|uninstall <port>" >&2; exit 2 ;;
esac
