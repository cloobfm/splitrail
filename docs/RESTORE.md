# Restoring splitrail on a new Mac

Splitrail is more than the binary: a config file, a state folder, hooks in four other tools, two
launch agents and a menu bar plugin. Migration Assistant brings most of it, but on the Nov 2026
move the dashboard came up with no machines in its top bar and only one account in Plans. Run this
list top to bottom on the new Mac, then the checks at the end.

## 1. What lives where

| What | Where | Notes |
|---|---|---|
| Config | `~/.splitrail.toml` | `[mqtt]` (broker, `name = "mbp"`), `[plans.*]`, `[goal]`. Holds the Cloud token and Slack webhook. |
| Secrets | `~/.config/splitrail/{mqtt_password,cursor_api_key,factory_api_key,warp_token}` | Not copied by the restore script; Migration Assistant brings them, or set them again. |
| Account limits | `~/.config/splitrail/limits/*.json` | One file per account (`main`, `v`, `cursor`, `droid`, `kiro`, `warp`, `antigravity`). Written by status lines and pollers, so a new Mac starts with only the ones it has seen run. |
| Plan history | `~/.config/splitrail/{plan_closes,plan_history,plan_values,records}.json` | How full each week got before it reset, plan changes, plan sizing, personal bests. Only on this Mac: losing them loses the history. |
| Logs splitrail keeps | `~/.config/splitrail/{cursor,antigravity,ollama,warp,logins,claude_sessions}/` | Hook and wrapper logs that the tools themselves do not keep. |
| Claude status lines | `statusLine` in `~/.claude/settings.json` (`… main`) and `~/.claude-v/settings.json` (`… v`) | Both run `scripts/claude_statusline_capture.py`; this is where Claude plan limits come from. |
| Cursor hooks | `~/.cursor/hooks.json` | `beforeSubmitPrompt`, `afterAgentResponse`, `stop` → `scripts/cursor_hook_capture.py`. |
| Wrappers | `~/.local/bin/{cursor-run,agy-run,warp-run}` | Headless runs that record usage. |
| Menu bar | `~/.swiftbar/splitrail.py` | Reads `~/.config/splitrail/status.json`. |
| Ollama proxy | `~/Library/LaunchAgents/com.splitrail.ollama-{proxy,env}.plist` | Proxy on 11434, Ollama.app on 11435. Copies in `scripts/`. |
| Pi | `~/.pi/agent/models.json` | `r4k` provider sends `X-Caller: pi`, so vLLM requests are not counted twice. |
| Tool transcripts | `~/.claude/projects`, `~/.claude-v/projects`, `~/.codex/sessions`, `~/.pi/agent/sessions`, `~/.cursor/projects/*/agent-transcripts`, `~/.gemini/antigravity-cli/conversations`, `~/.factory/sessions`, `~/.qwen`, `~/.copilot/history-session-state`, `~/.mtplx/logs` | The tools' own logs, which splitrail reads. Migration Assistant brought all but `~/.mtplx/logs` (local MLX request logs) in Nov 2026; step 5 finds any gaps. |
| Binary | `/opt/homebrew/bin/splitrail-dashboard` (+ `splitrail` link) | Built from this repo, then `codesign -s - -f --identifier com.splitrail.dashboard`. |

Other machines (devbox-a/c/d, gpu-1, ts677) publish to the broker on their own; nothing on the Mac
needs restoring for them to reappear, only a dashboard that has the `[mqtt]` config and password.

## 2. Steps

1. **Build and install** the dashboard from this repo (`cargo build --release`, codesign as above,
   link it into `/opt/homebrew/bin`).
2. **Check the config and secrets** came across: `~/.splitrail.toml` has `[mqtt]`, and
   `~/.config/splitrail/mqtt_password` exists. Set any that are missing.
3. **Quit the dashboard** if it is running. It rewrites the state files from memory and would
   overwrite what you restore.
4. **Restore the state folder** from the old Mac's Time Machine backup (or the old Mac itself):

   ```sh
   OLD="/Volumes/Backups of …/<latest>.previous/Data/Users/bzl/.config/splitrail"
   cp -a ~/.config/splitrail ~/.config/splitrail.before-restore-$(date +%Y%m%d-%H%M)
   python3 -I scripts/restore_state.py "$OLD" ~/.config/splitrail --dry-run
   python3 -I scripts/restore_state.py "$OLD" ~/.config/splitrail
   ```

   It copies what is missing, merges `plan_closes` / `plan_history` / `records` (highest figure
   wins) and the line logs (old lines kept, new ones added), and keeps the new Mac's copy of
   anything else. Secrets, prices and the status file are skipped.
5. **Transcripts**: list what the backup has that this Mac does not, per folder in the table,
   and copy back anything that is a log (not `.DS_Store`, locks or credentials):

   ```sh
   OLDHOME="/Volumes/Backups of …/<latest>.previous/Data/Users/bzl"
   for d in .claude/projects .claude-v/projects .codex/sessions .cursor/projects .factory/sessions .qwen .mtplx/logs; do
     echo "== $d"; comm -23 <(cd "$OLDHOME/$d" && find . -type f | sort) <(cd ~/"$d" && find . -type f | sort)
   done
   cp -pn "$OLDHOME/.mtplx/logs/"*.jsonl ~/.mtplx/logs/   # e.g.
   ```
6. **Status lines, hooks, wrappers, menu bar**: confirm each row of the table above exists. Start a
   Claude session under each config dir (`claude` and `CLAUDE_CONFIG_DIR=~/.claude-v claude`) once,
   so both status lines write fresh limits.
7. **Launch agents** (Ollama proxy): copy both plists from `scripts/` to `~/Library/LaunchAgents/`,
   then `launchctl bootstrap gui/$(id -u)` each one.
8. **Start the dashboard** (only now, so it starts with the config and password in place), then
   press `U` to bring every publisher to the same version.

## 3. Checks

- Top bar lists the publishers (devbox-a, devbox-c, devbox-d, gpu-1, ts677), each online.
- Plans shows every account: Claude Max ×2, the three Virtius Team seats, Codex, Cursor, Droid,
  Kiro, Warp, Antigravity.
- `r` on the Summary tab: 📅 Plan closes goes back weeks, not just since the move.
- `splitrail-dashboard quotas` lists the same accounts.
- "Skipping invalid entry" warnings at start-up name transcripts with a damaged line. Compare the
  file with the backup (`cmp`): if they match, the damage happened on the old Mac (in Nov 2026, two
  Claude transcripts had a block of zero bytes from a crash) and only those lines are lost.
- An Ollama request (`curl localhost:11434/api/tags`) goes through the proxy (`lsof -i :11434`
  shows python, `:11435` shows Ollama).
