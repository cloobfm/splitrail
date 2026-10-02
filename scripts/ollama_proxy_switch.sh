#!/bin/bash
# Move Ollama.app to 127.0.0.1:11435 and put the Splitrail usage proxy on 11434.
#   bash scripts/ollama_proxy_switch.sh            # switch (rolls back by itself if a check fails)
#   bash scripts/ollama_proxy_switch.sh rollback   # undo
# Installs two launch agents: com.splitrail.ollama-env (re-applies OLLAMA_HOST at login, since
# `launchctl setenv` does not survive a reboot) and com.splitrail.ollama-proxy (KeepAlive).
# Ollama is down for a few seconds while it restarts.

DIR="$(cd "$(dirname "$0")" && pwd)"
LA=~/Library/LaunchAgents
USAGE=~/.config/splitrail/ollama/usage.jsonl

listener() { lsof -nP -iTCP:"$1" -sTCP:LISTEN 2>/dev/null | awk 'NR==2{print $1}'; }
waitfor() { for _ in $(seq 1 60); do [ "$(listener "$1")" = "$2" ] && return 0; sleep 1; done; return 1; }
restart_ollama() {
    # A plain quit signal: `osascript quit` can be cancelled by Ollama's quit confirmation or a
    # macOS automation prompt (that is what failed on the first run, error -128).
    killall Ollama 2>/dev/null
    for _ in $(seq 1 20); do [ -z "$(listener 11434)$(listener 11435)" ] && break; sleep 1; done
    # The app normally stops its server; make sure, so the new one can take the port.
    pkill -f "Ollama.app/Contents/Resources/ollama serve" 2>/dev/null
    for _ in $(seq 1 10); do [ -z "$(listener 11434)$(listener 11435)" ] && break; sleep 1; done
    open -a Ollama
}

rollback() {
    [ -n "$1" ] && echo "!! $1 -- rolling back"
    launchctl unload "$LA/com.splitrail.ollama-proxy.plist" 2>/dev/null
    launchctl unload "$LA/com.splitrail.ollama-env.plist" 2>/dev/null
    rm -f "$LA/com.splitrail.ollama-proxy.plist" "$LA/com.splitrail.ollama-env.plist"
    launchctl unsetenv OLLAMA_HOST
    restart_ollama
    waitfor 11434 ollama && echo "Ollama back on 11434" || echo "!! Ollama not on 11434; open the Ollama app"
    exit 1
}

[ "$1" = "rollback" ] && rollback

# With "Expose Ollama to the network" on, the app starts its server with OLLAMA_HOST=0.0.0.0 and
# ignores ours, so Ollama would come back on 11434. Check before touching anything.
DB=~/Library/Application\ Support/Ollama/db.sqlite
if [ "$(sqlite3 -readonly "file:$DB?mode=ro" 'select expose from settings' 2>/dev/null)" = "1" ]; then
    echo "Turn off Ollama > Settings > \"Expose Ollama to the network\" first, then rerun."
    exit 1
fi

# Lines Droid's first tests wrote into the real log are not usage; keep them aside.
[ -f "$USAGE" ] && mv "$USAGE" "$USAGE.droid-test-fixtures" && echo "Moved old test fixtures aside"

cp "$DIR/com.splitrail.ollama-env.plist" "$DIR/com.splitrail.ollama-proxy.plist" "$LA/"
launchctl load "$LA/com.splitrail.ollama-env.plist"
sleep 1
echo "OLLAMA_HOST=$(launchctl getenv OLLAMA_HOST)"

restart_ollama
waitfor 11435 ollama || rollback "Ollama did not come up on 11435"
echo "Ollama on 11435"
[ -z "$(listener 11434)" ] || rollback "11434 still taken by $(listener 11434)"

launchctl load "$LA/com.splitrail.ollama-proxy.plist"
waitfor 11434 Python || rollback "proxy did not bind 11434"
echo "Proxy on 11434"

curl -sf -m 10 localhost:11434/api/tags >/dev/null || rollback "/api/tags failed through the proxy"
echo "/api/tags through proxy: ok"
MODEL=$(curl -s localhost:11434/api/tags | /usr/bin/python3 -c \
    'import sys,json; m=sorted(json.load(sys.stdin)["models"], key=lambda x: x["size"]); print(m[0]["name"])')
curl -sf -m 300 localhost:11434/api/generate \
    -d "{\"model\":\"$MODEL\",\"prompt\":\"say hi\",\"options\":{\"num_predict\":5}}" >/dev/null \
    || rollback "/api/generate failed through the proxy"
echo "/api/generate ($MODEL) through proxy: ok"
echo "Logged:"; tail -1 "$USAGE"
pgrep -f "ssh -N .*11434" >/dev/null && echo "ssh tunnel still up" || echo "note: no ssh tunnel process found"
echo "Done. Undo with: bash $DIR/ollama_proxy_switch.sh rollback"
