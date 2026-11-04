#!/bin/sh
# At login (com.splitrail.ollama-env): move Ollama.app's server to 127.0.0.1:11435 so the usage
# proxy (com.splitrail.ollama-proxy) can own 11434.
#
# `launchctl setenv` does not survive a reboot, and Ollama.app, a login item, can start before
# this runs: it then serves on 11434 itself, the proxy exits with "Address already in use" and
# nothing local is logged (Nov 1-4, 2026). So after setting the variable, an Ollama that already
# took 11434 is restarted to pick it up, and the proxy started once the port is free.
/bin/launchctl setenv OLLAMA_HOST 127.0.0.1:11435

taken_by_ollama() {
    /usr/sbin/lsof -nP -iTCP:11434 -sTCP:LISTEN 2>/dev/null | awk 'NR > 1 { print tolower($1) }' | grep -q '^ollama'
}

# Give a login item that is still starting a moment to bind.
sleep 20
if taken_by_ollama; then
    /usr/bin/osascript -e 'quit app "Ollama"' 2>/dev/null
    i=0
    while taken_by_ollama && [ $i -lt 30 ]; do sleep 1; i=$((i + 1)); done
    /usr/bin/open -a Ollama
    sleep 5
    /bin/launchctl kickstart -k "gui/$(id -u)/com.splitrail.ollama-proxy"
fi
