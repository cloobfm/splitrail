# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [2.0.0-dash-0.65] - 2026-10-16

### Fixed
- **Warp's output tokens counted, so Warp adds to tk/s.** Every Warp token used to count as input
  (Warp recorded one total per model). Warp now also records `total_charged_usage` with input,
  output, cache read and cache write tokens per conversation, which the analyzer reads, shared
  across the conversation's models by their token counts; conversations from before that stay
  as input. CLI runs, which report only cost, get input and output tokens from what each model
  cost at its list price (none for a model without a known price).

## [2.0.0-dash-0.64] - 2026-10-16

### Fixed
- **Menu bar tk/s in the right colour on every display.** With two displays whose menu bars
  differ (light on one, dark on the other), SwiftBar shows one fixed image on both, so the title
  was white on the light one. The item is now a native status item of its own
  (`scripts/swiftbar/menubar.swift`, replacing `render_title.swift`), drawn at display time in
  each menu bar's text colour, never faded on the inactive display, like iStat-style meters. The
  SwiftBar plugin still starts it and streams it the title and menu, and prints nothing to
  SwiftBar while it runs (a plain text item if the helper cannot be built).

## [2.0.0-dash-0.63] - 2026-10-16

### Fixed
- **Warp CLI runs show while they run, not only once they end.** `warp-run` saves the run's
  usage so far every 30 seconds, and a run still going is dated when it was last saved, so the
  Warp column shows it active (instead of idle for the whole run) with its cost rising.

## [2.0.0-dash-0.62] - 2026-10-16

### Added
- **Warp CLI runs (`warp agent run`) in the Warp column.** The Oz CLI saves nothing locally (the
  Warp app's database holds only interactive conversations) and prints no usage, so those runs
  were missing. `warp-run` (same arguments as `warp agent run`; `scripts/headless_run.py`, beside
  `cursor-run` and `agy-run`) runs it as `ndjson` for the run id, prints what text mode would,
  then saves `warp run get`'s record (credits and cost per model; no tokens, Warp does not report
  them) to `~/.config/splitrail/warp/runs/<run id>.json`, which the Warp analyzer reads: the
  prompt plus one message per model. `warp-run --record <run id> --cwd <dir>` saves a past run.

## [2.0.0-dash-0.61] - 2026-10-16

### Added
- **Monthly tk/s goal and a pace dot in the menu bar.** `[goal] monthly_tks` (default 150) is
  the output rate to average over the calendar month, every machine together. `status.json`
  gains `month`: the average since the 1st and the rate the rest of the month needs to finish
  on the goal. Beside `tk/s` a dot shows both: its colour is the current speed against that
  needed rate (green at or above, amber at half or more, red below), and it is filled while the
  month's average is at or above the goal, a ring while behind. The menu explains it and lists
  the month's average, the rate needed and the tokens so far.

## [2.0.0-dash-0.60] - 2026-10-16

### Fixed
- **Menu bar tk/s stays readable on the display you are not using.** It was a template image,
  which macOS fades on every display but the active one; it is now drawn in the menu bar's own
  text colour, the way iStat-style meters are, so it is never fainter than them. The colour
  comes from the menu bar itself (a zero-width status item in the helper), not Dark Mode, since
  a light wallpaper keeps the menu bar text dark even in Dark Mode. A stale reading is drawn at
  half strength rather than a third.

## [2.0.0-dash-0.59] - 2026-10-15

### Added
- **Kiro's monthly credits** in the Month Limit and Login rows (the plan, e.g. `Kiro Free`).
  Every ten minutes `kiro-cli chat --no-interactive /usage` reports the credits used of the plan
  and the reset date, through Kiro's own sign-in; `kiro_limits` parses it into
  `~/.config/splitrail/limits/kiro.json`. Kiro has no weekly window.

## [2.0.0-dash-0.58] - 2026-10-15

### Added
- **Kiro CLI's new TUI is counted.** Kiro 2.x keeps sessions in `~/.kiro/sessions/cli/`
  (`<id>.json` with per-turn credits, tool uses and replies; `<id>.jsonl` with prompts) instead of
  `data.sqlite3`; `kiro_sessions` reads them, costing credits at $0.02 (Pro's rate).
- Classic Kiro chats in `conversations_v2` (29 here) are read alongside `conversations` (10).

### Changed
- **Menu bar item: a `tk/s` label over a fixed-width number**, like the other meters, instead of
  `⚡ 136 tk/s`, half the width beside the notch and steady as the number changes. It is drawn by
  `scripts/swiftbar/render_title.swift`, compiled once into `~/Library/Caches/splitrail/` and kept
  running (about 7 ms a redraw, only when the number changes).
- **The plugin streams** (`splitrail.py`, SwiftBar `streamable`): one process rereading the status
  file every second, rather than SwiftBar starting Python for each refresh. The dashboard writes
  the file every second, backing off if working it out would cost over 5% of a core.

## [2.0.0-dash-0.57] - 2026-10-15

### Added
- **Update progress.** `⏳ building 0.57 · 63% · 1:42` in the header while `U` builds, and per
  machine while publishers update (`dev-a 0.53⏳ 0:32 of ~1:50`, `dev-b 0.53⏳ queued`). The
  percentage is cargo's crate count (forced on for a pipe with `CARGO_TERM_PROGRESS_WHEN`); near
  the end of the list, where an incremental build spends its time, the timer is shown against how
  long the same update took last time (`~/.config/splitrail/update_durations.json`). Cargo's
  progress redraws are kept out of `update.log`.

## [2.0.0-dash-0.55] - 2026-10-15

### Added
- **Menu bar tk/s (SwiftBar).** The dashboard, and `splitrail-dashboard mqtt` on a Mac, rewrite
  `~/.config/splitrail/status.json` every 5 seconds: overall tk/s and today's cost across every
  machine, each tool's rate, cost and limits, and the other machines' publisher versions.
  `scripts/swiftbar/splitrail.5s.py` shows `⚡ 69 tk/s` in the menu bar (grey when the file is
  over a minute old) with a dropdown of tools, limit windows with reset times, devboxes, and
  "Open dashboard", which attaches the `splitrail` tmux session in a new Warp window (through a
  tab config) or Terminal, starting it if needed. Install with
  `ln -s "$PWD/scripts/swiftbar/splitrail.5s.py" ~/.swiftbar/`.

## [2.0.0-dash-0.54] - 2026-10-15

### Changed
- **Update and devbox versions moved to the header**, beside the dashboard's own version
  (`2.0.0-dash-0.54  dev-a 0.53  dev-b 0.53`), out of the Activity title, which had run out
  of width after MQTT.

## [2.0.0-dash-0.53] - 2026-10-15

### Added
- **Other machines' publishers in the Activity title**, after MQTT: `dev-a 0.53` green when it
  runs this build's publishing code, `0.45⬆` yellow when behind, `off` grey when offline. Each
  publisher now reports its version and `publisher_rev`, a fingerprint of the publishing code
  (`build.rs`: everything under `src/` but the TUI and self-update, plus `Cargo.lock` without the
  package's own version), so a dashboard-only change or a version bump does not flag them.
- **`U` updates them too.** With this dashboard current and a publisher behind, `U` reinstalls
  each out-of-date machine with `scripts/install_mqtt_publisher.sh` over SSH (`⏳` while it runs,
  `❌` if it fails, output in `~/.config/splitrail/update.log`).

## [2.0.0-dash-0.52] - 2026-10-15

### Added
- **Update in place.** When the source tree the dashboard was built from carries a newer version,
  the Activity title shows `⬆ 0.52 · U to update` (checked every minute). `U` builds it in the
  same profile (`⏳ building`, output in `~/.config/splitrail/update.log`), then the dashboard
  restores the terminal and replaces itself with the new binary: same arguments, same terminal or
  tmux pane, no manual restart.

## [2.0.0-dash-0.51] - 2026-10-15

### Added
- **Warp's monthly credits** in the Month Limit and Login rows, from what the Warp app caches
  in its preferences (`AIRequestLimitInfo`): credits used of the month's allowance and the reset
  time, refreshed every 5 minutes into `~/.config/splitrail/limits/warp.json`. No token needed.

### Changed
- **The Warp column reads Warp's own database** (`warp.sqlite` in its group container) instead
  of `warp_network.log`, which Warp stopped writing in May, and mitmproxy captures: credits,
  tokens per model and tool activity per conversation, prompts with their folder. Cost is the
  credits at the plan's price per credit. It is opened read-only in place (it is several hundred
  MB) and fully re-read only when its AI tables change.
- The daily startup check of the old captured Warp token is gone.

## [2.0.0-dash-0.50] - 2026-10-15

### Fixed
- **Quitting still typed mouse reports into the shell** (`35;66;90M35;...`). Raw mode was left
  before mouse reporting was turned off, and crossterm's capture reports every pointer movement,
  which the TUI never uses. Mouse reporting is now clicks and wheel only, and on exit it is
  turned off first and pending input discarded before the shell gets the terminal back.

## [2.0.0-dash-0.49] - 2026-10-15

### Fixed
- **A crash left no trace.** A panic's message went only to the terminal, where restarting the
  dashboard in the same window cleared it. It is now also written to `~/.splitrail.log` with the
  thread and a backtrace.

## [2.0.0-dash-0.48] - 2026-10-15

### Changed
- **Bursts past the top of a histogram burn like a flame.** A slot above its scale draws its full
  bar in blue, and at three times the scale or more in white, so a clipped burst is told apart
  from one that merely fills the bar.

## [2.0.0-dash-0.47] - 2026-10-15

### Fixed
- **The Activity histogram showed one dot at 200+ tk/s.** It was scaled to the tallest of 44
  slots while the normal title draws 25, so a burst 21 minutes back, off screen, set the scale,
  and a single burst on screen pressed every other bar down to `⡀`. Each width is now scaled to
  the bars it draws, to the 90th percentile of its active slots; a burst above that clips at full.

## [2.0.0-dash-0.46] - 2026-10-15

### Changed
- **Remote columns shorten the machine before the tool.** `Claude Code` on `devbox-a` is now
  `Claude@dev-a` rather than `CC@devbox-a`: the part of the machine name before its last `-` is
  cut to three characters, and the tool is only abbreviated if that still does not fit.

## [2.0.0-dash-0.45] - 2026-10-15

### Fixed
- **Remote columns were cut off.** `Claude@devbox-a` needs 15 characters and columns are 13, so
  a remote name that does not fit abbreviates the tool (`CC@devbox-a`, `OC@devbox-a`,
  `Cdx@devbox-a`), and only past that shortens the machine name in the middle.
- **Remote Claude columns had no login or limits.** Published limit snapshots now carry the short
  login name (`support·team`, never the address), and a remote Claude column shows its machine's
  login and plan limits, or a newer snapshot of the same account from any machine.
- `scripts/install_mqtt_publisher.sh` installs the Claude status line capture on the box (only if
  no `statusLine` is set) and takes `--share-previews` to send redacted last-message previews.

## [2.0.0-dash-0.44] - 2026-10-15

### Fixed
- **The file watcher watched the whole home directory.** A tool whose data directory does not
  exist (Antigravity, Cline and Kilo on a box without them) fell back to its nearest existing
  ancestor, which could be `~` or `~/.config`, watched recursively. On Linux that is an inotify
  watch per directory under home, every `node_modules` included, so `splitrail-dashboard mqtt`
  on a devbox never finished starting; on macOS every file change in home reached the
  dashboard. The fallback now stops short of home and the folders directly in it; a tool
  installed later is picked up on the next start.
- `scripts/install_mqtt_publisher.sh` left the broker unset when `config init` had already
  written an empty `[mqtt]` section.

## [2.0.0-dash-0.43] - 2026-10-15

### Added
- **Other machines roll up into the dashboard, over MQTT.** Every publisher (a dashboard, or
  `splitrail-dashboard mqtt` on a devbox) now also sends, retained: `splitrail/<host>/daily`
  (each tool's last 30 days of daily totals, at most once a minute), `splitrail/<host>/recent`
  (the last hour of messages, numbers only, merged into 30-second slots past the last two
  minutes, so a busy hour stays around 10 KB) and `splitrail/<host>/limits` (plan limit
  snapshots, with the login replaced by a hash). The dashboard subscribes to every other host
  and adds a column per remote tool (`Claude@devbox-a`): its daily totals count in the summary,
  and its recent messages drive tk/s, the sparklines and active/idle state. For an account used
  on several machines, the newest limit snapshot from any of them wins. `[mqtt] roll_up = false`
  turns the subscription off.
- **Optional transcript previews across machines**: `[mqtt] share_previews = true` sends each
  tool's newest message as one line of at most 120 characters, with key-like tokens (`sk-…`,
  `ghp_…`, JWTs, long hex) and values after `Bearer` or `token=` redacted. Off by default.
- `scripts/install_mqtt_publisher.sh <ssh-host> <broker> <user>`: installs the publisher on a
  Linux box as a systemd user service (nice 10, idle I/O, 25% CPU and 1 GB memory caps), from a
  static binary cross-built here.

### Fixed
- **MQTT payloads over 10 KB were refused** by the client library's default packet limit, which
  also dropped the connection; the limit is now 4 MB.

## [2.0.0-dash-0.42] - 2026-10-15

### Fixed
- **No "No price" warning for Kilo Code's free model.** `kilo-auto/free` was already costed at
  $0 but warned as unpriced; names ending in `/free` now count as free tiers, like `-free` and
  `:free`.

## [2.0.0-dash-0.41] - 2026-10-15

### Fixed
- **Activity histogram nearly empty at a steady 35 tk/s.** It counted input tokens too (a
  single large context dwarfed every reply) and its scale only grew, so one burst flattened
  everything after it. It now counts output tokens, like the tk/s beside it, scaled to the
  tallest bar on screen with a 20 tk/s floor. Any activity in a slot now shows at least the
  lowest dot, in the per-tool sparklines too (a small slot used to round down to blank).

## [2.0.0-dash-0.40] - 2026-10-15

### Added
- **All-tools activity histogram in the Activity title**, between tk/s and the notification
  countdown: the last 12½ minutes (21 in verbose mode) of every tool's tokens, in 30-second
  bars, newest on the right. It is the per-tool sparkline buckets summed, so it costs no extra
  pass over messages; its scale only grows, so a quiet start does not pin every later bar at
  full height.

## [2.0.0-dash-0.39] - 2026-10-15

### Fixed
- **MQTT status stuck at `offline` while publishing.** With two instances on one machine (the
  dashboard and `splitrail-dashboard mqtt`), the one that exited marked the machine offline
  and the other only said `online` on connect, so Home Assistant showed every sensor as
  unavailable (and a watch complication as 0). `online` is now sent with every update.

## [2.0.0-dash-0.38] - 2026-10-14

### Added
- **MQTT box in the Activity title**, beside Slack: ✅ connected to the broker, ⏳ connecting,
  ❌ unreachable or login refused, ⬜ no broker configured.

## [2.0.0-dash-0.37] - 2026-10-14

### Added
- **Live stats over MQTT, with Home Assistant discovery.** Set `[mqtt] host` and `username`
  in `~/.splitrail.toml` (password in `~/.config/splitrail/mqtt_password`) and the dashboard
  publishes every 15 seconds and on every change: retained JSON on
  `splitrail/<host>/state` (output tk/s over the last 15 minutes, today's cost and output
  tokens, overall and per tool, and each tool's 5h / weekly / monthly limit used %), and
  `online` / `offline` on `splitrail/<host>/status` (`offline` is also the last will, so a
  machine that sleeps shows as unavailable). Home Assistant gets one device per machine with
  sensors for the totals and for every tool used in the last 30 days or with limits. Any other
  MQTT client can read the same topics. `splitrail-dashboard mqtt` publishes without the
  dashboard, for running under launchd. Publishing never blocks: while the broker is
  unreachable messages are dropped, and discovery is resent on reconnect.

## [2.0.0-dash-0.36] - 2026-10-13

### Added
- **Antigravity transcripts, from agy's own conversation store.** `agy` keeps every
  conversation in `~/.gemini/antigravity-cli/conversations/<id>.db` (SQLite, protobuf rows).
  The new `antigravity_store` decodes it: prompts and replies for the transcript view, tool
  calls, and each model call's exact usage (input, output, cache read, thinking) with the model
  that made it (`gemini-3.8-flash-high` → `gemini-3.8-flash`). Field numbers were checked
  against `agy`'s own JSON usage: equal to the token over a 128-call run. Interactive and
  headless runs are covered alike, so the column no longer depends on the status line or
  `agy-run` for tokens; the status line log remains the fallback for conversations without a
  database, and the source of the weekly quota. Databases are read from a temporary copy (agy
  keeps them open in WAL mode) and re-read only when they or their WAL change. Project names
  are now the folder name, as in the other columns.
- **Cursor Agent CLI column (BZL-75).** `cursor-agent` keeps no readable usage on disk
  (transcripts are text only, the chat store is encrypted), but its `afterAgentResponse` and
  `stop` hooks receive the turn's model and input/output/cache token totals (undocumented).
  `scripts/cursor_hook_capture.py`, registered in `~/.cursor/hooks.json`, appends them to
  `~/.config/splitrail/cursor/usage.jsonl` (never the response text); the new analyzer turns
  each generation into one assistant message, so the two hooks of a turn are not double
  counted. A `beforeSubmitPrompt` hook adds the prompt, and `afterAgentResponse` the reply, as
  500-character previews for the activity feed (as the Ollama proxy does). Cost comes from the
  price table; turns where Cursor chose the model (`default` /
  `auto`) are counted at $0 as `cursor-auto`. Login shows the hook's `user_email`.
- **Cursor monthly usage and a 🗓️ Month Limit row.** Cursor bills by the month (no 5h or weekly
  windows). With a Cursor user API key in `~/.config/splitrail/cursor_api_key`, `cursor_limits`
  exchanges it for an access token as `cursor-agent` does (`/auth/exchange_user_api_key`) and
  polls `DashboardService/GetCurrentPeriodUsage` every 5 minutes into `limits/cursor.json`. The
  new Month Limit row shows the total % used and time to the billing-cycle reset; it also shows
  Droid's monthly window, which `droid_limits` already captured.
- **Real Cursor cost per turn.** The same poller saves Cursor's billed usage events
  (`DashboardService/GetFilteredUsageEvents`, 60 days, then incrementally) to
  `cursor/charges.jsonl`. Each CLI turn takes its conversation's events billed since the
  previous turn: their charged cents become the turn's cost (Auto turns were $0), and their
  summed tokens replace the hook/print counts, which leave out billed summarization and subagent
  requests (up to 24× lower on long turns). Events from conversations with no CLI turn (IDE,
  bots) stay out of the column.
- Free-tier model names (`…-free`, `…:free`, e.g. OpenCode Zen's `cat-jam-free`) are $0 without
  a "No price" warning, and Ollama's `moondream` (no hosted per-token equivalent; its cost is the
  measured electricity) no longer warns about a missing street price.
- **Headless runs: `cursor-run` and `agy-run`.** Cursor's token hooks and agy's status line
  only run interactively, so `cursor-agent -p` / `agy -p` (how other agents call them) leave no
  usage. Both report exact usage in the `result` of `--output-format json|stream-json`.
  `scripts/headless_run.py`, installed as `~/.local/bin/cursor-run` and `~/.local/bin/agy-run`
  (links to a copy in `~/.config/splitrail/bin/`), runs the stock binary in print mode (adding
  `-p`; for agy bound to the trailing prompt, since agy's `-p` takes it as its value), passes
  `json` / `stream-json` output through byte for byte, prints the final result for plain text
  exactly as text mode does, and appends usage to the file each analyzer already reads
  (`cursor/usage.jsonl`, `antigravity/usage.jsonl` with `source: "print"`) plus the raw stream
  to `<tool>/runs/<conversation id>.jsonl`. Recording never changes output or exit code. The
  stock `cursor-agent` / `agy` are untouched, so tool upgrades need nothing. agy's print output
  names no model: `--model` if given, else the model of the latest interactive agy session.
- Cursor's `Auto` / `default` model is matched case-insensitively.

### Fixed
- **"No price for model claude-sonnet-5.5".** agy names it `Claude Sonnet 5.5`, which normalizes
  to the dotted `claude-sonnet-5.5`; neither the built-in table nor the feed's hyphenated
  `claude-sonnet-5-5` matched. Added `claude-sonnet-5-5` ($2 / $10, cache write $2.50, read
  $0.10 per 1M, as the feed lists it) with the dotted alias, like Opus 5.5.
- **Antigravity headless runs counted twice, and their cache reads dropped.** agy's status line
  also fires during `agy-run`, so a run's status line deltas and its exact `print` line were
  both counted. Status line lines of a conversation up to its print line (after the previous
  one) are now dropped, and print lines' `cache_read_tokens` are counted and priced (Oct 13:
  17.7M cache-read tokens that were missing; output 191K → 97K without the double count).
- **A Claude login's 5h limit stuck at another login's 100%.** After `/login` switched `v`
  from the team to the personal `a@virti.us`, a session still holding the team's numbers wrote
  them under the personal login; "same window, keep the higher" then kept 100% for the rest
  of the window although live sessions reported 25%. The status line capture now lets a
  session's report replace the stored windows when its numbers just changed (it made an API
  call) or on its first report, unless that first report matches another login's sessions
  exactly. Repeats only fill windows nothing has reported.

## [2.0.0-dash-0.35] - 2026-10-13

### Fixed
- **OpenCode usage since January was missing.** OpenCode moved its data from JSON files into
  `~/.local/share/opencode/opencode.db` (SQLite): 1.x into `message`, 2.x into
  `session_message`. Splitrail only read the JSON files, which stop at 2026-01-24, so
  `opencode run` sessions (e.g. 107 turns today) never appeared. The analyzer now reads the
  database read-only, both tables, deduplicated by message id (2.x copied part of the 1.x
  history), with project names from `session`/`session_v2` and tool calls counted from 2.x
  content parts. It watches `opencode.db-wal` so new turns show up live, and reads
  incrementally: nothing when the database and WAL are unchanged (OpenCode's own log writes in
  that folder trigger reloads), otherwise only rows whose `time_updated` moved.
- With the database present the 13,564 legacy JSON files (all of which it already holds) are
  no longer read, so OpenCode loads in about 2 s instead of reading every file at startup.
- Messages whose `summary` was a bool or lacked a title were dropped; the field is now
  ignored.

## [2.0.0-dash-0.34] - 2026-10-13

### Added
- **Droid 5h/Week Limit rows.** Droid's status line carries no usage and Droid stores none
  locally; its `/usage` panel reads Factory's `GET /api/billing/limits`. With a Factory API key
  in `~/.config/splitrail/factory_api_key` (mode 600) or `FACTORY_API_KEY`, the dashboard polls
  that endpoint every 5 minutes and writes `limits/droid.json` in the status line snapshot
  format, so the Droid column's limit rows and health use it. The monthly window and Extra
  Usage balance are saved too. A 5-hour window with nothing used yet shows 0% with no
  countdown. No key, no polling.
- **Login row for every limited column.** Codex shows its ChatGPT plan from the logs'
  `rate_limits.plan_type` (`chatgpt·plus`; the logs carry no email), Droid shows `factory`
  (Factory's API returns ids only), and Ollama shows this machine's short hostname.

## [2.0.0-dash-0.33] - 2026-10-11

### Fixed
- **Dashboard pinned 3-4 cores while Codex ran.** Codex appends to its live session file
  continuously; each append (file events every ~2 s, plus the 5 s poll) re-parsed every Codex
  session on disk (201 files, 279 MB here). The Codex analyzer now caches each parsed file by
  size and mtime and only re-parses files that changed. Claude Code already had an incremental
  cache; the other analyzers are tracked for the same treatment.

## [2.0.0-dash-0.32] - 2026-10-11

### Fixed
- One email can be on a personal plan and a team, each with its own limits; 0.31 keyed limits
  by email alone and mixed them. Logins are now keyed by email plus organization (from
  `oauthAccount.organizationType`/`organizationName`), and team logins show as `name·team` in
  the Login row and the status line. Logins unseen for 8 days are dropped from the snapshot.
- 0.31's idle-session check ignored *any* repeated report, so a login with no entry yet (e.g.
  after the key change) stayed blank, with no 5h/week limit, until usage moved by 1%. Only a
  report repeated across a login switch is ignored now.

## [2.0.0-dash-0.31] - 2026-10-11

Plan limits per login, for a Claude config directory that switches between accounts.

### Fixed
- Cycling `/login` between accounts in one config directory (e.g. three emails on `Claude (v)`)
  mixed their windows in one snapshot: whichever account had the later reset stuck, and the
  others' usage was dropped as "stale". The status line now reads the directory's current login
  from `.claude.json`, keeps limits per login, and shows the current one's. A session that has
  not called the API since the switch repeats the previous account's numbers; a report identical
  to that session's last one is no longer attributed to anyone.

### Added
- **Login** row on the dashboard: the account the 5h/week limits belong to.
- The Claude status line names the login (`v:support · …`) and ends with the other logins'
  weekly usage (`7d a 50%, b 12%`), to pick which account to switch to.

## [2.0.0-dash-0.30] - 2026-10-01

Ollama tracking that works end to end: a real streaming proxy, the port switch, street-price
cost, measured electricity, transcripts, and capacity in the limit rows.

### Fixed
- The 0.29 proxy returned **empty bodies** for `/api/generate` and `/api/chat`, streamed or not
  (it iterated the ints of a bytes object, and the non-streamed branch never wrote), handled one
  request at a time, and its tests wrote fixtures into the real `usage.jsonl`. Rewritten: streams
  as it receives (first byte 13 ms live), threaded, keep-alive, `HEAD`, errors passed through,
  usage logged before the response ends; tests use a fake upstream and a temp dir.
- The Ollama analyzer was never registered; it is now.
- Zero-token records (sessions that never ran a model, e.g. failed OpenCode runs on a local
  provider) no longer raise "No price for model": nothing to price means no lookup.

### Added
- `scripts/ollama_proxy_switch.sh` moves Ollama.app to `127.0.0.1:11435` and the proxy onto
  `127.0.0.1:11434`, installing `com.splitrail.ollama-env` (re-applies `OLLAMA_HOST` at login)
  and `com.splitrail.ollama-proxy` (KeepAlive). Checks every step and rolls back on failure;
  `… rollback` undoes it. Ollama.app ignores `OLLAMA_HOST` while "Expose Ollama to the network"
  is on, so the script refuses to run until it is off. It quits Ollama with `killall`, since an
  AppleScript quit can be cancelled. Clients, including the devbox ssh tunnel, are unchanged.
- Ollama cost is the **street price**, like every other tool (their subscriptions are not
  per-token either): Ollama names map to hosted ones (`qwen3.8:27b-mlx` -> `qwen3.8-27b`,
  `gpt-oss:latest` at 20.9B -> `gpt-oss-20b`, size from `/api/tags` logged by the proxy) and are
  priced at the median across providers in the feed (`price_feed::median_price`, free tiers
  excluded). `[pricing.overrides."<ollama name>"]` wins.
- `🔌 Energy` row: electricity actually used. The proxy runs `macmon` (no root) and splits each
  second's SoC power across the requests running then, logging `energy_j`; measured 12-19 W on
  this M1 Max. Costed at `[ollama] electricity_per_kwh`; `power_watts` is the fallback when no
  measurement exists. New `Stats.energy_wh` / `energy_cost`.
- Ollama's `⏳ 5h` / `📅 Week` rows show capacity: output tokens generated in the window against
  what the machine would generate running nonstop at its measured throughput (output tokens per
  busy second, prompt processing included), e.g. `19% of 23K`.
- Transcripts: the proxy keeps the caller's last message and the start of the reply (500
  characters, `OLLAMA_PROXY_PREVIEW_CHARS`), so Ollama appears in the Activity section.

## [2.0.0-dash-0.29] - 2026-10-01

Ollama token tracking (BZL-71).

### Switch steps (requires short downtime — user approves before applying)
1. Stop the current inference clients (other tools, ssh tunnel) briefly.
2. Move Ollama to 11435: `launchctl setenv OLLAMA_HOST http://127.0.0.1:11435` (or edit
   the Ollama launch agent's `OLLAMA_HOST` env) and restart Ollama.
3. Install proxy: `cp scripts/com.splitrail.ollama-proxy.plist ~/Library/LaunchAgents/`
   then `launchctl load ~/Library/LaunchAgents/com.splitrail.ollama-proxy.plist`.
4. Confirm proxy on 11434: `curl -s http://localhost:11434/api/tags | head`.
5. Confirm clients (including devbox ssh tunnel) can reach 11434 without reconfig.
6. Rollback: `launchctl unload ...`, remove plist, restore `OLLAMA_HOST` to 11434, restart Ollama.

### Added
- `scripts/ollama_usage_proxy.py`: Python 3 stdlib proxy (`ThreadingHTTPServer` + `urllib`).
  Listens on `OLLAMA_PROXY_HOST:OLLAMA_PROXY_PORT` (default 127.0.0.1:11434), forwards to
  `OLLAMA_UPSTREAM` (default http://127.0.0.1:11435) without changing Ollama's native port
  (other tools and an ssh tunnel depend on it). Passes `/api/generate` and `/api/chat` NDJSON
  and `/v1/chat/completions` / `/v1/completions` through unchanged; reads `prompt_eval_count`,
  `prompt_eval_cached_count`, `eval_count`, and `total_duration` from the final `done: true`
  chunk (or `usage` in non-streaming / OpenAI streaming with injected `include_usage`).
  Each inference writes one atomic line to `~/.config/splitrail/ollama/usage.jsonl`.
  Logging failures never break requests.
- `scripts/com.splitrail.ollama-proxy.plist`: launchd agent (`KeepAlive`, logs to `~/.splitrail.log`).
  Not installed by default; install/rollback steps in `CHANGELOG` only.
- `src/analyzers/ollama.rs`: reads the proxy's `usage.jsonl`. One assistant message per line,
  `Application::Ollama`, cost fixed at $0.0 (local inference is free; skips
  `calculate_total_cost` to avoid price warnings). Each line is its own session (Ollama has no
  conversation id); malformed lines skipped.

### Fixed
- `upload.rs`: skips `Ollama` uploads (cloud does not know the application).
- `notifications.rs`: `Application::Ollama` arm in `format_application`.

## [2.0.0-dash-0.28] - 2026-10-01

Antigravity CLI (`agy`) tracking.

### Added
- Antigravity column: sessions, tokens, and cost per day, plus `⏳ 5h Limit` / `📅 Week Limit`
  rows and health. `agy` keeps no readable usage logs (each conversation is a SQLite file of
  protobuf blobs), so `scripts/agy_statusline_capture.py` runs as its status line (configured in
  `~/.gemini/antigravity-cli/settings.json`) and records what the payload carries: each growth
  in a conversation's token totals goes to `~/.config/splitrail/antigravity/usage.jsonl`
  (running totals kept in `state.json` beside it, under a file lock), and the per-model quota
  goes to `~/.config/splitrail/limits/antigravity.json` in the Claude snapshot format, merged by
  the same per-window rules. Quota buckets resetting within a day fill the 5h row, later ones
  the weekly row, keeping the fullest of each.
- `agy` often sends only a display name (`Gemini 3.8 Flash (High)`); it is reduced to the feed
  slug (`gemini-3.8-flash`) for pricing.
- Cost is an upper bound: the totals do not separate cache reads, so all input is priced at the
  full input rate. Events without a model are counted under `antigravity-unknown` at $0.
- Usage only accrues while the status line runs; history before this release is not available.

### Fixed
- Status line temp files orphaned when the process is killed mid-write (seen under `agy`, which
  restarts the command on rapid state changes; one Claude `.main.*.tmp` too) are swept after 60s.

### Changed
- Uploads skip Antigravity messages, since Splitrail Cloud does not know the `antigravity`
  application yet.

## [2.0.0-dash-0.27] - 2026-10-01

Codex CLI plan limits on the dashboard.

### Added
- Codex CLI fills the `⏳ 5h Limit` and `📅 Week Limit` rows, and its health is the room left in
  the fuller window, like Claude accounts. Codex writes `rate_limits` into every token-count
  event; the parser now feeds those windows to `plan_limits::observe`, which keeps the newest
  observation per window across all sessions (order-independent, so parallel and repeated parsing
  are harmless). Windows are matched by `window_minutes` (≤1 day → 5h, ≥6 days → weekly) because
  Codex sometimes labels the weekly window `primary`. First live read: 5h 4% ↻4h30m, week 1% ↻6d17h.

### Removed
- The old Codex `rate_limits` parse into `Stats.rate_limits`. It expected camelCase window entries
  and failed on every current event (snake_case, plus `limit_id`/`credits`/`plan_type` fields), so
  the field was always empty.

## [2.0.0-dash-0.26] - 2026-09-28

Fewer unpriced models, and a Codex model-detection fix. Warnings panel on the author's data: 7 → 2.

### Fixed
- Codex CLI: newer Codex writes `turn_context.approval_policy` as an object. The typed parse
  expected a string, failed, and discarded the whole record including its `model`, so sessions
  whose model appears only there were priced as the `gpt-5` fallback (e.g. a realtime-voice
  session on `gpt-5.6-terra`). The model is now read from the payload independently of the typed
  parse, and `approval_policy` accepts any JSON.

### Added
- Loose price-feed matching, used only after the exact feed and built-in lookups miss: a name
  that matches the last segment of a provider-prefixed feed key (`qwen-3-32b` →
  `cerebras/qwen-3-32b`), or starts with one at a `-`/`:`/`_` boundary after dropping a `custom:`
  prefix (`custom:GLM-4.6-Coding-Plan-0` → `zai/glm-4.6`). The boundary rule keeps `gpt-5` from
  claiming `gpt-5.6-terra`. Results are memoised per model name. `pricing show` reports which
  feed key matched.

### Changed
- The unpriced-model warning now reads `No price for model X; costed at $0 (set
  [pricing.overrides."X"] if it is not free)` instead of `WARNING: Unknown model: X. Defaulting to $0.`

## [2.0.0-dash-0.25] - 2026-09-27

### Fixed
- Plan-limit rows flickered (weekly for one account read 5% → 24% → 9% → 16% within a minute).
  Every open Claude Code session runs its own status line and reports the limits from its own
  last API call, and the capture script let the last writer win, so idle sessions kept rolling the
  snapshot back. The script now merges each window under a file lock: for the same window the
  higher usage wins (usage only grows within a window), a reset time more than 30 minutes later
  starts a new window, an earlier one is a stale session and is ignored, and a payload missing a
  window keeps the known one (most payloads omitted `five_hour`, leaving it blank). The status
  line itself now shows the merged figures, so all sessions agree. Sampled live for 75s after the
  fix: one stable value per account.

- The dashboard slowly scrambled (text from alerts and message previews smeared across other rows)
  until the terminal was resized. With notifications on and Slack off, waiting-conversation alerts
  were printed to stderr on top of the running TUI; ratatui only repaints cells it thinks changed,
  so the stray text stayed. The file watcher, analyzer reloads, the Qwen parser and two TUI error
  paths did the same. They now go through `utils::report`, which sends messages to the Warnings
  panel and `~/.splitrail.log` while the TUI owns the terminal (stderr otherwise); alert text goes
  to the log. Message previews are passed through `sanitize_for_terminal`, which turns tabs,
  escape sequences and other control characters from transcripts into spaces.

### Added
- `scripts/test_claude_statusline_capture.py` (`python3 -m unittest scripts/test_claude_statusline_capture.py`).
- `Ctrl-L` forces a full repaint, for anything that still draws behind ratatui's back.

## [2.0.0-dash-0.23] - 2026-09-27

Plan-limit usage and reset times per Claude account (BZL-66), and health based on them (BZL-65).

### Added
- `scripts/claude_statusline_capture.py`: a Claude Code `statusLine` command that saves each
  account's `rate_limits` payload (5-hour and weekly `used_percentage` + `resets_at`) to
  `~/.config/splitrail/limits/<label>.json` and prints `label · model · 5h 23% ↻1h33m · 7d 16% ↻6d3h`.
  Configure it in each account's `settings.json` with that account's label; it never fails the
  status line.
- `plan_limits` module reads those snapshots. A window whose reset time has passed reads as 0%
  until the status line next runs; snapshots older than a week are ignored.
- Dashboard rows `⏳ 5h Limit` and `📅 Week Limit` show `used% ↻countdown` per Claude account,
  colored by fullness (`-` for tools without plan windows). The CLI table grows from 14 to 16 lines.

### Changed
- Health for a Claude account with a captured snapshot is the room left in its fuller window
  (`1 - max(5h, weekly) / 100`) instead of the fixed 250K/1.75M output-token targets. Other tools,
  and Claude accounts without a snapshot, keep the token estimate.

## [2.0.0-dash-0.22] - 2026-09-27

Claude Code now counts subagent work (BZL-63) and reads every account, each shown separately
(BZL-64).

### Fixed
- Subagent transcripts (`projects/<project>/<session>/subagents/agent-*.jsonl`) were never
  discovered: the pattern only matched `projects/*/*.jsonl`. They are now read and share their
  parent session's conversation hash and project, so their tokens, cost and tool calls count while
  Sessions still means one per conversation you started. On the author's data: 108 → 427 files,
  output tokens 65.1M → 74.2M (+14%), estimated cost $23,748 → $26,429, sessions unchanged at 105.

### Added
- Multiple Claude Code accounts. `[[claude_code.accounts]]` (`label`, `dir`) lists config
  directories; left empty, Splitrail uses `~/.claude` as "main" plus every `~/.claude-<label>` that
  holds both `projects/` and `.claude.json`. With more than one account, each gets its own
  analyzer and dashboard column, `Claude (<label>)`; a single account keeps the `Claude Code` name.
  `config show` lists the resolved accounts. The second account on the author's machine
  (`~/.claude-v`: 59 files, 35 sessions, 4.2M output tokens) was previously invisible.

### Performance
- The Claude Code conversation hash is computed once per file chunk instead of once per message.

### Removed
- `deduplicate_message_chunks`, unused since BZL-14 phase 2. `dedup_messages` is now test-only:
  it remains the batch reference that `Accumulator::fold` is tested against. `cargo build` is
  warning-free.

## [2.0.0-dash-0.21] - 2026-09-27

Model prices come from a live feed instead of only the hardcoded table, so new models are priced
without a release.

### Added
- `price_feed` module: fetches LiteLLM's public price list (`model_prices_and_context_window.json`)
  at most once a day and caches it at `~/.config/splitrail/prices.json`. Lookup order is
  `[pricing.overrides]` in the config, then the feed, then the built-in table in `models.rs`.
  Long-context surcharges (`*_above_{N}k_tokens`) become progressive tiers, matching the built-in
  tier math.
- A refreshed price that moves more than 10x from the previous feed (or from the built-in table,
  for a model the feed did not have before) is ignored and warned about, guarding against a bad
  upstream commit.
- `splitrail pricing refresh` fetches now; `splitrail pricing show <model>...` prints the price
  used and where it came from.
- Config: `[pricing] feed_enabled` (default true, also `config set pricing-feed-enabled`) and
  `[pricing.overrides."<model>"]` with `input_per_1m`, `output_per_1m`, optional
  `cache_write_per_1m` / `cache_read_per_1m`.
- Built-in price for `claude-opus-5-5` ($4 / $20, cache $5 write / $0.20 read), which was falling
  through to $0.

### Changed
- The dashboard waits up to 10s for the first-ever fetch, then refreshes a stale feed in the
  background; prices from a background refresh apply to messages parsed after it lands.
  `splitrail upload` waits up to 10s for a stale feed so uploaded costs use current prices.
  Costs already aggregated or uploaded are not re-priced.

## [2.0.0-dash-0.20] - 2026-09-07

BZL-14 phase 2 (in-memory half). The Claude Code analyzer folds parsed messages into per-day
aggregates as they arrive instead of retaining the corpus to re-aggregate on every reload.

### Changed
- `IncrementalJsonlCache` stores cursors, not contents. `CachedFile` no longer holds messages, and
  `refresh` returns only what it actually read this pass: nothing for an unchanged file, just the
  appended lines for a grown one. Callers fold what they are handed.
- `RefreshReport::needs_rebuild` reports that a file we had already consumed was rewritten, shrank,
  or vanished, invalidating folded state. A *first* parse never sets it — treating it as an
  invalidation would have made the initial load parse the whole corpus twice.
- `ClaudeCodeAnalyzer` keeps an accumulator (per-day sums, conversation start dates, per-day
  timestamps for active time, and dedup fold state) and `get_stats` reads aggregates from it.
  Raw messages are trimmed to the live window.
- `aggregate_by_date` is now built from `fold_entry_into_day` and `finalize_daily`, and the
  accumulator calls the same two functions, so the batch and incremental paths cannot drift.

### Performance
- Real corpus, four consecutive loads of all nine analyzers: RSS 337 → 395 → 429 → 468 MB before
  (~44 MB per reload, unbounded), 382 → 389 → 391 → 394 MB after (~4 MB per reload). Absolute RSS
  crosses over at the third load and diverges from there.

### Verified
- Total computed cost matches the previous implementation to $0.44 on $22,569.92 (0.002%), the
  residue of folding costs incrementally in f64 rather than summing once at the end.
- `incremental_accumulation_matches_a_cold_full_parse` folds a split message across separate
  refreshes and asserts the result equals a cold full parse.

### Note
- Only the Claude Code analyzer folds incrementally so far; the other eight still retain their
  messages and re-aggregate. They are a smaller share of the corpus but the same change applies.

## [2.0.0-dash-0.19] - 2026-09-07

### Fixed
- The 🔥 Streak row showed 30 for every tool ever used, however long ago. `aggregate_by_date` pads
  every day from a tool's first message through today with empty `DailyStats` so charts have a
  continuous series, and the streak counted rows whose *date* fell in the window without checking
  whether the day carried any activity. Verified against real data: Codex CLI, Qwen Code, and Kilo
  Code each had 0 active days in the last 30 while all three displayed 30.

### Added
- `days_with_activity_in_last_30`, extracted from the render path so the rule is testable, with
  tests covering the padded-days case and window boundaries.

## [2.0.0-dash-0.18] - 2026-09-07

Groundwork for BZL-14. This bounds what the *stats* structure retains; it does **not** by itself
reduce RSS, because the incremental cache still holds every parsed message. That is BZL-14 phase 2.

### Added
- `DailyStats::active_seconds` — time spent actively working on a day, the sum of gaps under 15
  minutes between consecutive messages, computed during aggregation. This was the only value the
  day drill-down derived from raw historical messages; message count and session count were
  already available as `user_messages + ai_messages` and `conversations`. Serialized with
  `#[serde(default)]`, so older persisted stats deserialize as 0. It is also a new field in the
  upload payload.
- `utils::retain_live_window` and `utils::live_window_cutoff`, applied in
  `AnalyzerRegistry::load_all_stats` and the watcher's incremental reload.

### Changed
- The day drill-down reads message count, session count, and active time from `daily_stats`
  instead of scanning the selected day's messages, so it no longer needs raw history resident.
- Only the last `LIVE_WINDOW_HOURS` (24 h) of raw messages are retained per analyzer. Measured on
  the real corpus: 127,577 retained messages becomes 6,933, with all 1,837 daily rows intact.
- `DailyStats`, `Stats`, and `RateLimitInfo` derive `PartialEq`.

### Fixed
- Trimming can never discard a message the uploader still owes the server: when uploading is
  configured and its watermark is further back than the live window, the cutoff moves back to the
  watermark.

## [2.0.0-dash-0.17] - 2026-09-07

### Fixed
- The incremental cache no longer deep-copies its messages on every refresh. `IncrementalJsonlCache::refresh` returned a freshly cloned `Vec<ConversationMessage>` on *every* path, including `Outcome::Unchanged`, so a reload that touched no files still re-allocated the entire corpus. It now hands back per-file `Arc<Vec<ConversationMessage>>` chunks, and an unchanged file costs a refcount bump instead of a copy. This is the clone that 0.15's `Arc` work did not reach: that change removed the copies *downstream* of the cache, while this one sat upstream of them.
- Deduplication consumes the shared chunks directly (`deduplicate_message_chunks`) rather than first flattening them into one large owned vector, removing another full copy of the corpus per reload.
- A refresh no longer emits empty chunks for missing or still-empty files.

### Performance
- Real corpus (128 files, ~70.8K messages, release build), RSS across six refreshes with no file changes: 490.7 MB held / 520.4 MB retained before, 305.5 MB held / 335.2 MB retained after. The prior ~2 MB-per-refresh creep is gone — RSS is now identical from the fourth refresh onward.

### Added
- `unchanged_file_hands_back_the_same_allocation_instead_of_a_copy` asserts that an unchanged file's messages keep their heap addresses across refreshes, so this regression cannot return silently.

## [2.0.0-dash-0.16] - 2026-09-06

### Removed
- Gemini CLI analyzer is no longer registered. Google ended Gemini CLI service for free, AI Pro, and AI Ultra accounts on 2026-06-18 in favor of Antigravity CLI, so no new data can arrive for individual users. The Gemini CLI tab, its reload work, and the `Gemini CLI sessions failed to parse` startup warning are gone. The analyzer source, the `GeminiCli` application variant, and Gemini model pricing stay in the tree; re-enable by uncommenting one line in `create_analyzer_registry`. See `docs/GEMINI_CLI_DEPRECATION.md`.

## [2.0.0-dash-0.15] - 2026-09-05

### Changed
- Analyzer stats are shared, not copied. `MultiAnalyzerStats` now holds `Arc<AgenticCodingToolStats>` per analyzer, so a reload replaces one entry and the watch channel, TUI, notifier, and uploader all reference the same allocation. Previously every reload cloned all ~118K retained messages three to six times (manager copy, channel copy, TUI copy, notifier copy).
- The per-CLI health column compared each message's date to today by formatting both to strings. It now compares dates directly. That formatting ran for every message on every redraw and was the hottest frame in the render profile.
- The incremental cache pre-sizes the merged message vector instead of growing it by doubling.

### Performance
- Same isolated benchmark as 0.14 (copy of real sessions, one assistant line appended every 2 s, release build, steady state): CPU 11.7% average with 30% peaks before, 7.7% average with 25% peaks after. Resident memory 863 MB before, 601 MB after; large heap allocations 742 MB before, 222 MB after.

## [2.0.0-dash-0.14] - 2026-09-05

### Changed
- Claude Code sessions are parsed incrementally. A new per-file cache (`src/incremental.rs`) remembers each file's size, mtime, parsed byte offset, and parser state. On reload, unchanged files are not opened, grown files are parsed from the previous offset with the project label carried over, and only fully newline-terminated lines are consumed so a half-written line is never parsed twice or half-parsed. Shrunk or rewritten files fall back to a full parse; files that disappear are dropped.
- `parse_jsonl_file` is now a thin wrapper over a resumable `parse_jsonl_chunk` that reports bytes consumed.

### Performance
- Reloading no longer re-parses the entire 1.2 GB Claude Code corpus on every file write. Isolated benchmark with a copy of the real sessions and one assistant line appended every 2 s: steady-state CPU 198.7% average with 524% peaks before, 11.7% average with 30% peaks after.
- Known trade-off: the cache keeps parsed messages resident, so RSS rose from about 670 MB to about 860 MB in that benchmark. Removing the duplicate copies is tracked separately (BZL-8).

## [2.0.0-dash-0.13] - 2026-09-05

### Changed
- Codex CLI is only re-parsed when one of its files actually changed. The 5-second poll now compares a fingerprint of every source file's path, size, and mtime (one `stat` per file, no reads) instead of unconditionally re-parsing all sessions on disk.

### Performance
- Steady-state CPU in an isolated environment with only Codex data: 17.6% average with 179% peaks before, 1.4% average with 2.8% peaks after

## [2.0.0-dash-0.12] - 2026-09-05

### Fixed
- Test suite is green again: 64 unit tests pass (was 55 pass, 7 fail, 1 ignored) and the integration target compiles and passes 7 tests. The failures were stale expectations behind intentional changes (tool-result turns dropped, Droid session-total message, aggregate gap-filling and local-date bucketing), plus a HOME-override race between integration tests, now serialized behind a lock with a guard that restores HOME on drop.

## [2.0.0-dash-0.11] - 2026-09-05

### Fixed
- Ctrl-C now quits the TUI. Raw mode delivered it as a key event that nothing handled, so the only way out was to kill the process.
- The terminal is restored on every exit path: normal quit, error, panic, and SIGTERM/SIGHUP/SIGINT. A drop guard plus a panic hook disable raw mode, mouse capture, and the alternate screen and show the cursor. Killing the app no longer leaves the shell printing `35;col;rowM` mouse-tracking sequences on every mouse move.
- If a signal arrives while the event loop is blocked in a long reload, the terminal is restored directly after 3 seconds and the process exits

## [2.0.0-dash-0.10] - 2026-09-05

### Fixed
- Claude Code parser no longer rejects lines with unfamiliar record types (`attachment`, `cost-state`, `last-prompt`, `mode`, `permission-mode`, `worktree-state`, and others) or unfamiliar content block types (`tool_reference` inside ToolSearch results, `fallback`). Both enums gained a `#[serde(other)]` catch-all.
- Removes roughly 266,000 "Skipping invalid entry" warnings that permanently occupied the TUI warnings panel during active sessions
- Token usage on assistant messages that contain an unmodeled block is now counted instead of discarded

## [2.0.0-dash-0.9] - 2026-09-05

### Added
- 2026 frontier model pricing: 48 models and 133 aliases covering Claude 4.6 through Fable 5.1 and Mythos 5.1, GPT-5.1 through GPT-6 Astra (including the GPT-5.6 Sol/Terra/Luna family and GPT-5.3-codex), Gemini 3 Flash through 3.8 Flash and 3.1 Pro, Grok 4.20 through 4.6 and Grok Build, DeepSeek V4, Kimi K3 and K2.x, GLM 5.x and 4.7, MiniMax M3 and M2.5, Qwen 3.8 Max and the 3.5 family
- Short aliases `opus`, `sonnet`, `haiku`, `fable`, and the missing `claude-opus-4-5` alias; `codex-auto-review` priced as gpt-5.3-codex
- Unit tests for model resolution and Fable 5.1 cache-read pricing

### Fixed
- Every model released since late 2025 was silently priced at $0

## [2.0.0-dash-0.8] - 2025-11-29

### Added
- **Droid CLI (Factory AI) support**: Complete analyzer for Factory AI's Droid CLI usage data
  - Comprehensive token tracking including input, output, cache creation, cache read, and thinking tokens
  - Session metadata extraction (working directory, duration, autonomy mode, reasoning effort)
  - Project context detection from working directories
  - Cost calculation using model-specific pricing for Claude Opus 4.5, Sonnet 4, GPT-5, and more
  - Full schema documentation with JSON validation files and sample data
  - Integration with Splitrail TUI and dashboard alongside existing AI tools
- **Model pricing support**: Added pricing for Claude Opus 4.5 (`claude-opus-4-5-20251101`) with Anthropic-style cache pricing
- **Data source coverage**: Droid CLI sessions stored in `~/.factory/sessions/*/` with JSONL format for conversations and JSON for settings

### Changed
- Enhanced application enum with `DroidCli` variant for proper categorization
- Updated notifications module to display "Droid" as the friendly name for Droid CLI data
- Extended analyzer registry to include Droid CLI analyzer in the default tool set

## [2.0.0-dash-0.7] - 2025-11-24

### Added
- Dual stacked bar charts for tokens/day with separate scales for total and output tokens
- Braille character rendering (⢀⢠⢰⢸⣸⣼⣾⣿) for smoother chart visualization
- Phase 2 CPU optimizations: batched file watching to reduce analyzer reloads (20-30% reduction during high file activity)
- Phase 1 CPU optimizations: adaptive polling with dynamic rates based on user activity (50-75% idle CPU reduction)
- Lazy clock updates and optional clock disable via SPLITRAIL_DISABLE_CLOCK environment variable
- xAI Grok 4.1 Fast model pricing and aliases
- CPU measurement scripts and benchmark guide
- Incremental aggregation infrastructure for future optimizations

### Changed
- Improved tokens/day chart with Y-axis scale, better contrast, and fixed width
- Split tokens/day chart into dual displays: total tokens/day (blue) and output tokens/day (green)
- Updated internal pricing for qwen3 and gpt-oss models
- Enhanced chart X-axis with 7-day interval markers and date labels
- Switched from block characters to smooth braille characters for chart rendering
- Reduced Y-axis width from 4 to 3 characters for more horizontal space
- Added Min/Max/Avg statistics for each chart
- Implemented batched processing with 500ms window for file events
- Optimized polling rates: 1000ms when idle, 250ms when active

### Fixed
- Chart bar alignment and cleaned compiler warnings
- UTF-8 panic in verbose mode message truncation
- Streak calculation to show exactly 30 days (not 31)
- Fixed indentation errors in models.rs pricing definitions

## [2.0.0-dash-0.6] - 2025-11-22

### Added
- Waiting-for-input notification module with Slack webhook delivery and TUI status indicators (countdown + last-send)

### Changed
- Notification delivery now sends only one alert per analyzer per check and ignores long-stale sessions (over-threshold by >15 minutes)

## [2.0.0-dash-0.5] - 2025-11-22

### Fixed
- OpenCode analyzer now properly displays actual conversation content for both user and assistant messages
- Fixed OpenCode schema to include summary.body field for message content
- Fixed timezone conversion for OpenCode daily stats (UTC to local time)
- OpenCode messages now read from part files to show real conversation text instead of action descriptions
- OpenCode role detection now properly distinguishes between user and assistant messages
- Live activity feed now shows meaningful OpenCode conversations matching other CLI patterns

## [2.0.0-dash-0.4] - 2025-11-20

### Fixed
- Health bar visualization now depletes from left to right (fills right to left) for more intuitive display
- Updated braille partial characters to fill from right side first
- Maintains 104-dot granularity (13 chars × 8 dots) for smooth percentage representation

## [2.0.0] - 2025-11-10

- Add support for Cline, Kilo Code, Roo Code (Cline forks) and Qwen Code (Gemini CLI fork) (#25 and #26) - @bl-ue
- Various fixes and improvements to Splitrail Cloud

## [1.2.0] - 2025-10-27

- Correct Claude Code token aggregation for split messages (#23) - @bl-ue

## [1.1.3] - 2025-10-24

- Add support for Claude Haiku 4.5 to the Claude Code analyzer (#21) - @bl-ue

## [1.1.2] - 2025-10-01

- Add support for Claude Sonnet 4.5 to the Claude Code analyzer
- Ignore the "file-history-snapshot" entry added in Claude Code 2.0.0

## [1.1.1] - 2025-09-18

- Fix Codex CLI (#16) - @bl-ue

## [1.1.0] - 2025-09-18

### Added
- Enhanced Codex CLI support with updated analyzer functionality
- Better formatting and lint compliance

## [1.0.1] - 2024-09-14

### Added
- Download link in README for binary releases

### Changed
- Rearranged README structure for better readability
- Improved documentation and project presentation

### Fixed
- Various formatting and lint error corrections

## [1.0.0] - 2025-08-09

### Added

- **Initial stable release** of Splitrail
- Real-time automatic uploading to Splitrail Cloud
- Comprehensive multi-tool support:
  - Claude Code
  - Gemini CLI
  - Codex CLI
- Rich Terminal User Interface (TUI) using ratatui
- Advanced cost calculation and token usage analytics
- File operation tracking (read/write/edit operations with byte/line counts)
- Comprehensive model support:
  - Claude models (Sonnet 4, Opus 4, Opus 4.1)
  - GPT models (GPT-5 series, GPT-4 series)
  - Gemini models (2.5-pro, 2.5-flash, legacy 1.5 series)
  - Codex CLI models (o3, o1 series, gpt-4.1 series)
- Configuration management with TOML-based config
- Splitrail Cloud integration with API token authentication
- Deduplication logic to prevent duplicate entries
- Real-time file watching capabilities
- Parallel processing for performance
