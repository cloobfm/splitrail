#!/bin/sh
# Open the dashboard from the menu bar: attach to the `splitrail` tmux session in a new terminal
# window (Warp through a tab config, else Terminal), starting the session (release build) if it
# is not running.
repo=$(cd "$(dirname "$0")/../.." && pwd)
tmux=$(command -v tmux || echo /opt/homebrew/bin/tmux)
"$tmux" has-session -t splitrail 2>/dev/null ||
    "$tmux" new-session -d -s splitrail -c "$repo" "cargo run --release; exec zsh"

if [ -d /Applications/Warp.app ]; then
    # Opened by file name: warp://tab_config/<file name without .toml>.
    config="$HOME/.warp/tab_configs/splitrail_dashboard.toml"
    mkdir -p "$(dirname "$config")"
    cat > "$config" <<TOML
# Written by splitrail's menu bar plugin (scripts/swiftbar/open_dashboard.sh).
name = "Splitrail dashboard"

[[panes]]
id = "main"
type = "terminal"
directory = "$repo"
commands = ["$tmux attach -t splitrail"]
TOML
    open "warp://tab_config/splitrail_dashboard?new_window=true"
else
    osascript -e "tell application \"Terminal\" to do script \"$tmux attach -t splitrail\"" \
        -e 'tell application "Terminal" to activate' >/dev/null
fi
