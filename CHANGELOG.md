# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [2.0.0-dash-0.123] - 2026-10-23

### Fixed
- The 🗓 Plans timeline's right-hand columns did not line up: the reset mark `▏` is two cells
  wide in some terminal fonts, and a monthly plan's date (`Oct 31`) was shorter than a weekly
  one's (`Sat 10:59`). The mark is now `│` (one cell everywhere) and the date column has one
  width.
- A legend under the bars: `⣿` full plan, `⣤` Team seat (a smaller weekly allowance), `⣀`
  full and waiting for its reset, `│` the reset, `»` resets after this week.

## [2.0.0-dash-0.122] - 2026-10-23

### Changed
- The 🗓 Plans timeline draws in braille dots like the Summary's charts instead of solid
  blocks, which ran together into one slab: `⣿` for a full plan, `⣤` (lower half) for a Team
  seat, `⣀` for a plan that is full and waiting for its reset.

## [2.0.0-dash-0.121] - 2026-10-23

### Fixed
- "No price for model gemma4:31b / gpt-oss:120b / nemotron-3-*" warnings, and those requests
  counted twice. While testing models, Claude Code was pointed at Ollama (its Anthropic-
  compatible endpoint): the replies landed in Claude (main) at $0, and again in the Ollama column
  through its proxy. Claude columns now skip replies on models this Mac's Ollama has served
  (from its usage log, re-read when it changes); the Ollama column keeps them with their
  transcript and price, and they stay out of the Claude plan's numbers. Of 171 such replies in
  Claude (main), 64 are in this Mac's Ollama log; the rest went to another Ollama before its
  proxy existed and are no longer counted.
- `gemma4:31b` had no hosted price: price lists write `gemma-4-31b`. A single-word name with a
  whole-number version is also tried hyphenated (`gemma4` -> `gemma-4`; `qwen3.8` is left as is).

## [2.0.0-dash-0.120] - 2026-10-23

### Added
- Each Claude login's plan is recorded and shown: the status line capture reads Claude Code's
  own account record (`organizationType`, `seatTier`, rate-limit tier, extra usage) into
  `Max 20x`, `Max 5x`, `Team Std`, `Team Prem`, `Pro`. The timeline and quota views name it
  per row. Here: Claude (main) is Max 20x; Claude (v)'s two Team logins are Team Std (not
  Premium).
- Plan changes are noticed: `plan_history.json` keeps every account's plan over time (Claude
  logins, Cursor's and Warp's monthly limits, Kiro's plan and credits, Codex's ChatGPT plan),
  checked every 30 s, with a "📈 Plan changed" notification and a log line on an upgrade, a
  downgrade or a new credit limit. A newly seen account is recorded, not announced.

## [2.0.0-dash-0.119] - 2026-10-23

### Added
- 🗓 Plans timeline under "Where to spend next" (`p` on the Summary tab): every plan on one
  shared 7-day axis, each bar running from now to its reset, soonest first, so deadlines line
  up instead of being read off "4d15h" one at a time. Coloured by the share left (green 50%+,
  yellow 20-50%, red below; grey while full and waiting), a white mark at the reset, `»` for a
  monthly plan that resets past the week (with its date), Team seats drawn as a shorter bar
  (`T`), and each row ending with its % left, reset time and the Do action. In the bordered,
  bold-titled style of the Summary's charts.
- The dashboard's own quota view now lists a Claude account's other logins too (it built its
  list apart from `status.json`), and names the signed-in login on its row.

## [2.0.0-dash-0.118] - 2026-10-23

### Added
- Claude usage is credited to the login it ran under (BZL-283). One account directory (Claude
  (v)) is used with a Max plan and Team seats, each its own weekly allowance, and switching
  mid-week used to mix them. The status line capture now keeps a permanent login history
  (`~/.config/splitrail/logins/<label>.jsonl`, a line per change). `/login` switches the whole
  directory at once, so each reply belongs to the login the directory was on at its time.
  Each hour/model/project slice of a day now also carries that login.
- "Where to spend next" measures each login against its own week, to the hour: the signed-in
  row counts its own usage (plus anything from before history was kept), and another login's
  row gets its own spend once it has been used, so a Team seat's quota is measured instead of
  guessed. History starts on 2026-10-23, seeded from the sessions the capture already knew.

## [2.0.0-dash-0.117] - 2026-10-23

### Added
- "Where to spend next" lists every login a Claude account has used, not only the one signed
  in now: a Max plan and Team seats behind one config directory are separate weekly allowances.
  Each other login is its own row (`Claude (v) · a@Virtius`) whose Why reads "switch with
  /login · N% free". A weekly window resets at the same time each week, so a login not used
  since its reset counts as a whole fresh week (and ranks as USE NOW), with its next reset known.
- The status line capture keeps logins for a year instead of 8 days, which had already
  forgotten the Team seats. Each login is recorded the next time it runs a prompt.

## [2.0.0-dash-0.116] - 2026-10-21

### Fixed
- Slack messages kept coming after Slack was switched off. The sender was built once at start,
  and `s` (or editing `[notifications.slack] enabled`) only changed the config file, so a
  dashboard started with Slack on posted until it was restarted: here from 02:37 to well after
  it was switched off at 10:27. The switch now takes effect at once, and the notifier re-reads
  the setting on every check (15 s), so a hand edit counts too.

## [2.0.0-dash-0.115] - 2026-10-21

### Added
- `?` opens a panel with every keyboard shortcut, grouped by where it works (everywhere, the
  Summary tab, tool tabs); `?` again or Esc closes it. The help bar starts with "?: all keys",
  so it stays findable when the rest is cut off on a narrow terminal. A test checks that every
  key the dashboard handles is in the panel.
- The help bar names the Slack key (`s: Slack`) instead of the unclear "Next - Last - Slack".

## [2.0.0-dash-0.114] - 2026-10-21

### Fixed
- The dashboard panicked ("index out of bounds: the len is 2 but the index is 2", tui.rs) when a
  status message was shown: it was drawn into a third part of a help bar that has two. Present
  since the Slack toggle's message, but rarely hit until 0.106 made Esc at the top level show
  "Press q to quit", so the first Esc there crashed it. It is drawn over the right-hand part
  now. A screen test draws a status message and fails with the old line.

## [2.0.0-dash-0.113] - 2026-10-21

### Added
- `U` updates a Windows publisher (vrig) as well as Linux ones. `scripts/update_publisher.sh`
  asks the host what it is over ssh and hands Linux to `install_mqtt_publisher.sh` and Windows
  to `scripts/windows/update_windows_publisher.sh`, which cross-builds the `.exe`, stops the
  running one (Windows locks it), copies the files compressed and reruns the installer. The
  Ollama proxy task is left as it is, on or off, and not restarted, so an update never drops
  other machines' requests. Checked on vrig: 52 s end to end.
- `[mqtt.ssh]` maps a publisher's name to the ssh host to reach it by, where they differ
  (`vrig = "VRig"`: ssh aliases are case-sensitive, and `vrig` went to the wrong host).

## [2.0.0-dash-0.112] - 2026-10-21

### Added
- A Windows GPU box as a publisher (vrig: Windows 10, RTX 4000 Pro, Ollama).
  `scripts/windows/install_windows_publisher.ps1 -Name vrig -Broker … -User …` installs the
  cross-built `splitrail-dashboard.exe` as a windowless logon task publishing `mqtt` under its
  `[mqtt] name`, writes the config through the program's own `config init` (a partial file does
  not parse; written without a byte-order mark, which Windows PowerShell adds and TOML
  rejects), and registers the usage proxy (`ollama_proxy_vrig.pyw`: :11434 on the LAN, Ollama
  behind it on 127.0.0.1:11435) as a second logon task, left disabled until Ollama moves.
  Installed on vrig: the publisher is online as `vrig 0.111`. The proxy was tested on a spare
  port against its Ollama: tags, a generation, tokens, model size and family.
- The proxy measures GPU board power with `nvidia-smi` where there is no `macmon` (summed over
  cards; the CPU is not counted).
- Requests from other machines (a shared GPU box) carry the caller's address as their
  `#project`, where no process can be looked up.
- `install_mqtt_publisher.sh` refuses a host that is not Linux, so `U` cannot push a Linux
  binary at the Windows box.

## [2.0.0-dash-0.111] - 2026-10-21

### Fixed
- The Windows build compiles, for the first time: cross-compiled from the Mac with
  `cargo zigbuild --target x86_64-pc-windows-gnu --features rusqlite/bundled` (24 MB `.exe`).
  The one error was the TUI's quit-signal listener, which used Unix signals; on Windows it waits
  for Ctrl-C. Not yet run on Windows.

## [2.0.0-dash-0.110] - 2026-10-21

### Added
- `[mqtt] name = "mbp"` sets this machine's name on the broker and in other dashboards'
  columns (`Claude@mbp`), so no network or rename can change it. Unset, it is the Local
  Hostname on macOS (0.109), else the short hostname. After changing it, clear the old name
  with `splitrail-dashboard mqtt --forget <old name>`.

## [2.0.0-dash-0.109] - 2026-10-21

### Fixed
- This Mac counted twice: its MQTT name came from `hostname -s`, which follows the network.
  When it changed from `Amberts-MacBook-Pro` to `Amberts-MBP` (a DHCP name), the data published
  under the old name stayed on the broker and showed as another, offline machine (`Amb-Pro`)
  with its own `Claude@…` columns: this Mac's own history again. Totals, the monthly pace and
  the records (biggest day, week, month, day cost, goal streak) counted it twice from about
  23:49 on Oct 20. On macOS the name is now the Local Hostname from Sharing
  (`scutil --get LocalHostName`), which does not move with the network.
- `splitrail-dashboard mqtt --forget <host>` clears what a renamed or retired machine left on
  the broker (state, status, roll-up payloads and its Home Assistant sensors).

## [2.0.0-dash-0.108] - 2026-10-21

### Fixed
- 0.107 shipped with a failing unit test. Its new `/api/show` lookup ran inside tests and asked
  whichever Ollama was installed, so a test that expects no price found one. The lookup is off
  in tests. Also `status_file::tests::totals_every_tool…`, which failed in the first minute after
  local midnight (its "60 seconds ago" message landed on yesterday), now keeps recent messages
  on today. 452 tests pass, including at 00:00:55.

## [2.0.0-dash-0.107] - 2026-10-20

### Fixed
- `minimax-m3.1-flash` costed at $0: MiniMax M3.1 Flash (launched 2026-10-19; Claude Code was
  pointed at it today) is not in the price feed yet. Added to the built-in table at its
  published $0.15 in / $0.60 out per 1M, $0.015 cached input.
- Ollama `qwen3.8-64k` costed at $0. Two causes: Ollama's `/api/tags` names it
  `qwen3.8-64k:latest`, so the proxy's size lookup by the bare name missed (it now tries
  `:latest` too), and a context-length variant (`-64k`) matched no hosted name (it is now priced
  as its model, `qwen3.8-27b`). Lines logged without a size borrow one from another line of the
  same model, or ask Ollama's `/api/show` once per run (`/api/tags` leaves size and family empty
  for MLX models).
- "Price feed changed azure/gpt-4o-mini-audio-preview… by more than 10x" no longer shows on
  screen: the feed lists thousands of models, almost none in use, and the guard keeps the
  previous price anyway. It goes to the log, and `pricing refresh` still reports it.

## [2.0.0-dash-0.106] - 2026-10-20

### Changed
- Esc goes back one level instead of quitting. On the Summary tab it closes records or quotas,
  then verbose; on a tool tab it clears a model/project filter, then returns to day rows, then
  to the Summary tab. With nothing left to close it says "Press q to quit". `q` and Ctrl-C
  quit as before; the help line reads "Esc: back, q: quit".
- Publishers are copied compressed (`scp -C`): 11.5 MB becomes about 5 MB on the wire.
  Measured to devbox-a over a slow link (about 20 KB/s): 8-10 minutes plain, 3-3.5 compressed,
  for 0.6 s of compression on the Mac.

## [2.0.0-dash-0.105] - 2026-10-20

### Added (BZL-238, part 3 of 3)
- Local model servers show what agents ran on them. In the verbose view, the Ollama and MTPLX
  rows add the rate of agent requests on their models (`🖥 1.0 tk/s via Pi Agent`) and those
  requests' lines in their transcript, tagged `via Pi Agent`, with the agent's project. So a
  busy Pi on mtplx no longer leaves the MTPLX row empty.
- Shown only: each request is still counted in one column (the agent's, which has the
  transcript and project), so tk/s, cost, the monthly goal and records add up as before. An
  agent request is recognised by its model being one the server has run (`qwen-fast`,
  `qwen3.8:27b-mlx`); other machines' columns are left out (`src/servers.rs`).

## [2.0.0-dash-0.104] - 2026-10-20

### Added (BZL-238, parts 1 and 2 of 3)
- Which project a local model request came from. The usage proxy finds the calling process
  from each connection (`lsof`, once per connection, off the request path) and logs its name
  and working directory (`caller`, `cwd`). Every local caller is 127.0.0.1, so before this
  every Ollama row had the same `#project`; now mediasrc's jobs read `#mediasrc`. Lines from
  before the change keep the old tag. The proxy also logs the reply's own `id` (`response_id`).
- The same proxy can front mtplx: `scripts/mtplx_proxy.sh install 8000` puts it on :8000 (a
  launchd agent) and expects mtplx on :18000 (`mtplx start pi --port 18000`); clients keep
  using :8000. Its log (`~/.config/splitrail/mtplx/usage-<port>.jsonl`) is merged into the
  MTPLX column by request id: the prompt and reply text, the calling project and measured power
  for requests that do not come from Pi. Checked end to end against the running mtplx: the
  proxy's reply id is mtplx's `request_id`, and the caller came back as `python3` in
  `~/Projects/mediasrc`.
- MTPLX models are found by alias in Pi's `models.json` (`qwen-fast` → "Qwen 3.6 35B-A3B"),
  not by port, so pricing still works with mtplx moved behind a proxy.
- The Ollama proxy needs a restart to start logging callers:
  `launchctl kickstart -k gui/$(id -u)/com.splitrail.ollama-proxy`.

## [2.0.0-dash-0.103] - 2026-10-20

### Fixed
- Two (or more) tk/s items in the menu bar: SwiftBar restarts the streaming plugin now and then
  (waking, refreshing) without stopping the copy it had, and that copy, printing nothing to
  SwiftBar, never noticed, so it kept its own native item. Three copies were running here, one
  3.5 days old and orphaned. The newest copy now records its pid in
  `~/Library/Caches/splitrail/swiftbar-plugin.pid`; an older one that finds another pid, or that
  has been orphaned, exits within a second and its item goes with it.

## [2.0.0-dash-0.102] - 2026-10-20

### Changed
- Pi on MTPLX models is counted once, in the Pi Agent column, with its transcript (BZL-236).
  0.100 left Pi's mtplx replies to the MTPLX column, so a running Pi showed no tk/s and neither
  column had any text. Pi's `responseId` is mtplx's `request_id`, so each request has one home:
  Pi Agent counts Pi's own (priced and metered as MTPLX would: street price of the model behind
  the provider's port, electricity from mtplx's wall time), and MTPLX keeps the rest: OpenCode,
  the web UI, scripts and benchmarks through the server. Checked on real data: all 48 of Pi's
  mtplx replies match a request in mtplx's log; 13 other requests stay under MTPLX.
- Pi Agent keeps message text (prompts and replies, not thinking; up to 2,000 characters), so
  the verbose view shows its transcript.

## [2.0.0-dash-0.101] - 2026-10-19

### Added
- MTPLX costs (BZL-236), priced like Ollama's: `cost` is the street price, the median across the
  hosted providers of the same model (Qwen 3.6 35B-A3B: $0.199 in / $1.10 out per 1M over 8
  providers; Qwen 3.8 27B: $0.425 / $3.00 over 4), and electricity goes in `energy_wh` /
  `energy_cost` from each request's wall time at `[ollama] power_watts` and
  `electricity_per_kwh`. The log names only the server's alias, so the model comes from Pi's
  `models.json` for that port ("Qwen 3.6 35B-A3B (local, fast)" → `qwen3.6-35b-a3b`;
  quantisation words dropped). `[pricing.overrides."qwen-fast"]` wins over both.

## [2.0.0-dash-0.100] - 2026-10-19

### Added
- MTPLX column (BZL-236): local models served by mtplx (`mtplx start pi`, one port per model:
  qwen-fast 35B-A3B on 8000, qwen-deep 27B on 8001, qwen-code on 8002), read from mtplx's own
  `~/.mtplx/logs/request-log-<port>.jsonl` and its rotations. One reply per request: input
  (prompt minus cached), cache read, output; $0 cost; conversations by `session_id`. Written by
  the server, so Pi, OpenCode, the web UI and the terminal are all covered (Pi saves no sessions
  in this setup). Warm-up requests, which name no model, count under the port's model. A
  benchmark run inside mtplx rather than through its server is not logged and cannot be seen.
- Pi Agent skips replies from `mtplx-*` providers, so MTPLX use is never counted twice.
- Not uploaded to Splitrail Cloud, which does not know it.

## [2.0.0-dash-0.99] - 2026-10-18

### Fixed
- The dashboard leaked a zombie process per desktop notification (BZL-208): the notifier
  (`osascript`, `notify-send`) was started and never waited on. One running since Oct 17 had
  3,349. It is now waited on in the background. Zombies already there clear only when the process
  exits, so restart it rather than pressing `U` (which replaces the program in place).
- "🏆 New record" fired every 30 s while today, this week and this month were all the best so
  far: each check "beat" them by the tokens since the last one, three notices a time, about six
  a minute. A record still growing in its own day, week, month or slot now updates silently; one
  is announced when a new period beats an older one.

## [2.0.0-dash-0.98] - 2026-10-18

### Fixed
- Other machines' columns showed one line in the verbose activity view, against five for local
  tools: publishers shared only their newest message's text. They now send the last 5 messages
  with text (`texts` in the recent payload, redacted and cut like before). Messages older than
  two minutes are merged into time buckets on the wire, so their texts arrive as entries of their
  own with no tokens; counts and tk/s are unchanged. Shows once the publishers run 0.98.

## [2.0.0-dash-0.97] - 2026-10-18

### Fixed
- Other machines' columns (`Claude@dev-a`) showed no `#project` in the verbose activity view: the
  roll-up feed carried no project. Publishers now send it (`p`), so it shows once they run 0.97;
  older publishers still leave it blank.
- Ollama embeddings were not counted: the usage proxy logged chat and generate calls only, so
  mediasrc's `qwen3-embedding` job passed through unseen. `/api/embed`, `/v1/embeddings` and the
  legacy `/api/embeddings` are now logged with their input tokens (no output, so they never add
  to tk/s). The proxy has to be restarted to pick this up:
  `launchctl kickstart -k gui/$(id -u)/com.splitrail.ollama-proxy`.
- "Droid limits: Failed to reach Factory … operation timed out" flashed on screen after a single
  timed-out poll (about 1 in 300). A failed poll is now retried after a minute and logged; the
  warning only appears after 3 failures in a row.

## [2.0.0-dash-0.96] - 2026-10-18

### Added
- Hourly rows and model/project filters on the tool tabs (BZL-138, the rest of it after 0.84's
  week/month rows). `g` now cycles day → week → month → hour. `f` steps through the tab's models
  and `o` through its projects, busiest first, then back to all; `x` clears both. A filter
  applies to every period, and the help line says what is showing.
- Each day now keeps its tokens, cost and replies by hour, model and project (`DailyStats.slices`,
  only the combinations that occurred: about 3,700 across all tools here), so both work over all
  of history, not just the live window. Filtered and hourly rows carry tokens and cost only; the
  activity columns stay empty. Claude's folded corrections move the slice with the day.
- Checked on real data: every tool's slices add up to its daily output and cost on every day.

## [2.0.0-dash-0.95] - 2026-10-18

### Added
- Claude Code's own session cost, recorded for checking splitrail's sums against (BZL-193). The
  status line capture now keeps each session's running `cost.total_cost_usd`, which Claude Code
  adds up itself from the API responses, in `~/.config/splitrail/claude_sessions/<account>.json`
  (with the transcript path, 45 days kept). The status line's `context_window` token counts are
  not running totals (they fall back after each turn), so tokens are not taken from it.
- First checks, by hand: Claude (main) daily tokens and cost match `ccusage claude daily` exactly
  for Oct 11-16. One session's transcripts sum to $172.90 against Claude Code's $182.30, about
  5% under; calls that never reach the transcript (compaction, titles) are the suspects, to be
  confirmed by the reconcile command.

## [2.0.0-dash-0.94] - 2026-10-18

### Fixed
- Claude's live tk/s read about 2.3x too high (BZL-192): 855 tk/s where the real figure was 354.
  Claude Code writes one JSONL line per content block (thinking, text, tool_use), each repeating
  the reply's usage. Since 0.20's incremental fold (BZL-14, 2026-09-07) the daily totals folded
  those lines together, but the live-window messages kept every line. Everything that sums
  messages counted a reply once per block: "Now" tk/s (TUI, status.json, menu bar, MQTT),
  today's cost and output in `tool_states`, the roll-up feed to other machines, and the Records
  peak tk/s. The live window now holds one message per reply with the folded stats.
- Daily, weekly and monthly totals, cost, Left/Need and the other Records were never affected
  (they come from the folded daily totals); nothing was uploaded to Splitrail Cloud.
- The inflated Records peaks are dropped and re-derived: the two peak kinds are saved under new
  names, and records whose kind is unknown are skipped on load instead of discarding the file.

## [2.0.0-dash-0.93] - 2026-10-18

### Fixed
- Gemini CLI: 23 of 169 saved sessions failed to parse (BZL-10). Newer CLIs write `info` messages
  ("Request cancelled.") and user prompts as a list of `{"text"}` parts; both now parse, recovering
  their messages. The analyzer stays unregistered (retired 2026-09), so this only matters if it is
  re-enabled.

## [2.0.0-dash-0.92] - 2026-10-18

### Fixed
- **Droid CLI session totals were uploaded repeatedly (BZL-190).** The session-total message's
  `global_hash` embedded `last_timestamp` — the session's *newest* message — so it changed every
  time the session gained a message. The server deduplicates on `global_hash`, so each changed
  value read as a new message and was inserted again, and that row carries the session's whole
  token usage from `settings.json`. A session active across N uploads contributed its full totals
  N times, overstating uploaded Droid cost and tokens.

  The identity is now the session alone, run through `hash_text` like every other analyzer's:
  `hash_text(&format!("droid-cli:{session_id}:session-total"))`. Repeated uploads are idempotent.

  Local figures were never affected — within a single run the timestamp is fixed, so dedup worked.
  This only ever bit on upload, and only with `auto_upload` enabled, which is why it never showed
  as a visible wrong number.

  The per-message hash (`droid-cli:{session}:{message}:{ts}`) was already stable and is unchanged.

### Note
- Rows already uploaded keep their old identities, so this does not retract duplicates the server
  has stored. If `auto_upload` was ever on with Droid sessions active, server-side Droid totals are
  already inflated; cleaning that up is a separate decision for the user and deliberately not
  attempted here.

## [2.0.0-dash-0.91] - 2026-10-17

BZL-130 upstream sync. The triage found almost nothing left to port — see the ticket — so this is
the one item worth taking.

### Added
- `utils::format_number_fit` formats a number to fit a column width, falling back from the
  configured format to the abbreviated form (`13.38b`) and then to a decimal-less abbreviation,
  rather than letting ratatui clip. The per-CLI and per-period numeric cells
  (`Constraint::Length(13)`) use it via a single `NUMERIC_COLUMN_WIDTH` constant so the width and
  the formatting cannot drift apart.

  Ported in spirit from upstream #124 and #201, not as their diff: our `tui.rs` has diverged too
  far for theirs to apply.

### Note on severity
This is **latent, not live**. No value in the current `status.json` has a comma form wider than 13
characters, so nothing is being misrendered today. It is worth fixing because ratatui clips with no
marker: a 14-character total would render as a smaller, entirely plausible number, which is the
worst failure mode for a tool whose job is reporting cost. A 30-day window of 10B+ tokens reaches
it. (The 13.4B figure that prompted the check was an all-time sum across 60 Codex session files,
not a cell the TUI draws — worth stating so the next person does not re-derive it as a live bug.)
## [2.0.0-dash-0.90] - 2026-10-17

### Added
- **📅 Plan closes, in the Records view (`r`).** How full each plan's week or month was when it
  reset, per account. Claude's two machines on one login count as one account.
  - Columns: the last close, the best one, the average of the last four, and the current window
    so far.
  - Each figure is the **highest** one seen in that window. Idle sessions keep re-reporting the
    figure from their last API call, so the latest report can be lower than where the week
    really ended. Claude (main)'s week ending Oct 17 read 90% that way, when it was about 93%.
  - Kept in `~/.config/splitrail/plan_closes.json` (26 windows per account and plan).
  - A notification when a window closes: "📅 Claude (main) week closed · 93% used, 7% left
    unused".

## [2.0.0-dash-0.89] - 2026-10-17

BZL-12, second pass: the behaviour Linux needs, not just the build.

### Added
- **Linux desktop notifications.** A new record posts through `notify-send` (libnotify) on Linux,
  AppleScript on macOS, and nothing anywhere else. The platform choice sits behind a `Notifier`
  trait whose implementations *build* the command rather than run it, so each is unit tested
  without spawning a process — including that Linux posts nothing when `notify-send` is not on
  `PATH`, and that macOS escapes the text through `serde_json` so a quote in a record cannot break
  the AppleScript.
- **Polling fallback when the OS runs out of file watches.** Linux caps inotify watches per user
  (`fs.inotify.max_user_watches`), and a tree the size of `~/.claude/projects` can exhaust it —
  after which every further watch fails and the dashboard sits there looking live while seeing
  nothing. `notify` reports that as `ErrorKind::MaxFilesWatch`; hitting it now switches to
  `PollWatcher` at a 5-second interval and prints the `sysctl` command that fixes it properly.
  `FileWatcher` holds a `Box<dyn Watcher + Send>` so the kind can be chosen at runtime.
- GitHub Copilot's analyzer offers Linux (`~/.config/...`), VSCode-server and Windows (`%APPDATA%`)
  paths, matching the shape Cline, Kilo Code and Roo already had. Still unregistered, so this is
  groundwork rather than a live fix.

### Note
- The watch-limit *detection* is unit tested (`MaxFilesWatch` triggers the fallback; a missing path
  or generic IO error does not, so one bad directory cannot demote the whole watcher to polling).
  The fallback *itself* has not been exercised against a real exhausted inotify — that needs a
  Linux box with the limit lowered.
- 408 tests pass. Linux target still reports zero Rust errors.

## [2.0.0-dash-0.88] - 2026-10-17

BZL-12, first pass: the build blockers, re-audited against 0.87 rather than the 0.16 audit in the
ticket.

### Fixed
- **SQLite is bundled.** `rusqlite` had no `bundled` feature, so `libsqlite3-sys` linked against
  the system library via pkg-config/vcpkg: Ubuntu needed `libsqlite3-dev` present, musl would not
  link at all, and Windows needed a vcpkg sqlite3. It now builds SQLite from source.
- **Three unix-only call sites are gated**, each with a Windows path rather than a hole:
  `plan_limits::short_hostname` used `libc::gethostname` (Windows reads `COMPUTERNAME`);
  `tui::TerminalGuard::restore` used `libc::tcflush` (POSIX only — the console is left as crossterm
  restores it); `self_update::exec_new_binary` used `std::os::unix::process::CommandExt::exec`
  (Windows has no exec, so it spawns the replacement and exits).
- **CI no longer requests `rustc-codegen-cranelift-preview`.** Nothing sets `codegen-backend`, so
  the component was never used — but it was requested on every leg including `windows-2025`,
  where the preview component is not reliably published, making it a build failure for no benefit.
- **`.cargo/config.toml` no longer pins `lld-link`** for `x86_64-pc-windows-msvc`, which required
  LLVM on the builder. CI already overrode it with `link.exe`, so the pin only affected local
  Windows builds, where the default MSVC linker needs no extra installs.

### Added
- `tests/cross_platform_paths.rs` discovers Linux (`~/.config/...`) and macOS
  (`Library/Application Support/...`) layouts from a fake home, asserts an `APPDATA` pattern is
  offered for Windows, and checks Kilo Code still offers the `vscode-server` layout a headless
  Ubuntu box uses. Analyzers emit every platform's patterns unconditionally and let the filesystem
  decide, which is what makes this testable from a Mac.

### Verification status
- **Linux: the Rust code compiles.** `cargo check --target x86_64-unknown-linux-musl` reports zero
  Rust errors; it stops only at the bundled-SQLite C step, which wants a musl cross-gcc that macOS
  does not have. A native Ubuntu build has `cc`, so this is a cross-compilation limit, not a code
  problem.
- **Windows: unverified.** The cfg gates are mechanical but no Windows target can be checked from
  here (same C-toolchain wall). CI is the first real test.
- **No Ubuntu smoke run.** `devbox-a` is reachable and is Ubuntu 6.8.0, but has no cargo, and
  installing a Rust toolchain on it was not something to do unasked.

### Still open on BZL-12
- Warp credits need macOS: `warp_limits` shells out to `defaults` for the cached
  `AIRequestLimitInfo`. It fails cleanly off macOS (the error propagates, nothing panics), so Warp
  simply reports no credit limit on Linux. A real fix needs another source for that number.
- `records.rs` posts notifications via `osascript`; the spawn is ignored on failure, so it is a
  silent no-op elsewhere. Linux would want `notify-send`.
- GitHub Copilot's analyzer is still macOS-only in its paths, but remains unregistered.
- inotify watch limits on Linux, and the Windows console/glyph questions, are untested.

## [2.0.0-dash-0.87] - 2026-10-17

BZL-14 phase 4.

### Changed
- The upload paths clone only the messages actually pending. `upload.rs`, `watcher.rs` and
  `main.rs` each flattened every retained message into one vector and *then* filtered it to those
  past the watermark — copying the whole corpus to keep a handful. `utils::messages_pending_upload`
  filters while collecting, and replaces `get_messages_later_than`, which was also `async` without
  containing a single await.

### Note on the other phases
- **Phase 3 (day drill-down re-parses on demand) needs no work.** The drill-down already reads
  `daily_stats` rather than raw messages, since 0.18 moved `active_seconds` into the aggregate.
- **Phase 2b (persist the ledger) is not worth building as specified.** Its two motivations have
  both been addressed by other work: startup is 4.32 s cold / 0.13 s warm (measured, all 11
  analyzers, release build) after the BZL-74 caches, and the memory it was meant to reclaim went
  with the incremental fold in 0.20. Persisting derived state across restarts would add a
  versioned on-disk format and a new way to report numbers from state not just read — the failure
  shape behind two of today's three real defects. Recorded on BZL-14 rather than built.

## [2.0.0-dash-0.86] - 2026-10-17

BZL-74, second pass: Kiro CLI, Ollama and Cursor Agent, plus companion-file stamping.

### Added
- `StampCache::parse_all_stamping` stamps files a parse reads *beside* each source. A source is
  often not the only file its result depends on, and stamping only the discovered path serves
  stale messages whenever the companion is what moved.

### Changed
- **Kiro CLI** parses through the cache for both of its source kinds: a TUI session's `.json` is
  stamped with the sibling `.jsonl` that carries its events, and the classic `.sqlite3` with its
  `-wal`/`-shm`, since SQLite commits land in the WAL before the database file changes. Its
  database and session parsing moved into `parse_database` / `parse_session_pair`.
- **Ollama** keeps its parsed usage log against (size, mtime). `usage.jsonl` is append-only and
  grows forever, so re-reading it every reload got steadily worse. Busy samples are cached beside
  the messages rather than recomputed, because `plan_limits::record_busy` replaces the whole set
  and `capacity_in` sums across all of it — handing it only a freshly parsed tail would quietly
  understate throughput.
- **Cursor Agent** stamps each usage log together with the charges file it is priced against, so a
  change to charges re-prices every entry.
- **Droid CLI** now stamps the sibling `.settings.json` too, closing the staleness gap noted in
  0.81: settings carry the authoritative token counts, so a settings-only rewrite must invalidate.

### Performance
Real data, cold then warm parse of the same sources (release build):

| analyzer | files | messages | cold | warm | speedup |
| --- | --- | --- | --- | --- | --- |
| Kiro CLI | 3 | 1,724 | 27.1 ms | 2.0 ms | 13.3x |
| Ollama | 1 | 8,946 | 35.8 ms | 1.7 ms | 21.0x |
| Cursor Agent | 1 | 44 | 2.1 ms | 1.8 ms | 1.2x |
| Qwen Code | 81 | 5,861 | 17.4 ms | 2.3 ms | 7.4x |
| Droid CLI | 64 | 3,498 | 22.1 ms | 1.0 ms | 21.1x |
| Kilo Code | 112 | 3,164 | 37.6 ms | 1.6 ms | 23.7x |

Totals identical across both passes for every analyzer. Cursor Agent's small factor is just its
size — one file, 44 messages — not a cache miss.

### Corrections to the BZL-74 audit
- **Antigravity and Warp already cache** (`CachedConversation` per database; a stamped
  `Arc<Vec<_>>` respectively). The audit's "none" for both was out of date, so no work was needed.
- **True cursor mode for Ollama is still worth having** but is not a drop-in: busy samples are
  derived from whole-file content, so an incremental reader has to accumulate them rather than
  parse a tail. The stamp cache removes the idle-reload cost; the growing-file cost remains on a
  first read.

### Still open on BZL-74
Spec item 4 (on-disk cache persistence) and the CPU acceptance run. Item 3 (coalescing reload
triggers) landed in 0.85.

## [2.0.0-dash-0.85] - 2026-10-17

### Fixed
- **One reload per change, and none lost (BZL-74 item 3).**
  - The 5-second Codex poll used to reload Codex itself. A write that both it and the file
    watcher saw was parsed twice. The poll now queues Codex into the same batch as file events,
    and a Codex reload refreshes the poll's fingerprint, so the next poll finds nothing new.
  - An analyzer reloaded within the last 2 s used to have its event dropped. The final write of
    a burst then stayed unseen until some unrelated write came along. It is now kept pending and
    reloaded once the debounce passes.

## [2.0.0-dash-0.84] - 2026-10-17

### Added
- **Weekly and monthly rows on the tool tabs (BZL-138).** `g` cycles each tool tab between
  days, ISO weeks (`2026-W42`) and months (`2026-10`). Rows merge the daily totals, carrying
  the 1h cache-write subset through. The totals row says what it counts (`Total (6mo)`), and
  the help line shows the current grouping.

### Fixed
- **Cached-token totals were cut off.** The column was 13 wide, so totals of 10B or more lost
  their leading digits (24.7B showed as "4,741,882,109"). It is now 15 wide.

## [2.0.0-dash-0.83] - 2026-10-17

### Fixed
- **`test_claude_code_analyzer_with_test_data` no longer reads the real `~/.claude`.** It built
  the analyzer before overriding `$HOME`, and `ClaudeCodeAnalyzer::new()` reads HOME when it is
  constructed, so discovery scanned the live corpus. Two parses of it disagreed whenever a
  session was writing. The test now passes its temp config directory to `for_account` and
  asserts that only that corpus is found. The unused `HomeOverride` helper is gone.

## [2.0.0-dash-0.82] - 2026-10-17

### Added
- **Copilot CLI and Pi Agent columns (BZL-135, upstream #141 / #65).**
  - **Pi Agent** reads `~/.pi/agent/sessions/*/*.jsonl`, with Pi's own token counts and its
    reported cost (our prices when it reports none). Replies that failed before any tokens are
    skipped.
  - **Copilot CLI** reads 0.0.3xx's `~/.copilot/history-session-state/session_*.json`. Those
    files record no tokens and no model, so tokens are **estimated** at about 4 characters a
    token: output is the reply plus its tool calls; input is what is new since the last reply,
    with the earlier conversation as cache reads. The model is taken as Claude Sonnet 4.5, the
    CLI's default then. Newer CLIs' `session-state/<id>/events.jsonl` (real usage, upstream #141)
    is not read yet, since none exist here.
  - Both use the per-file `StampCache`. Neither is uploaded to Splitrail Cloud, which doesn't
    know them; Kilo CLI (0.78) is now kept out of uploads for the same reason.

## [2.0.0-dash-0.81] - 2026-10-17

BZL-74, first pass: the shared whole-file cache, and the four analyzers whose sources are replaced
rather than appended to.

### Added
- `src/stamp_cache.rs` — `StampCache`, a per-file parse cache keyed on (path, size, mtime) for
  analyzers whose sources are rewritten rather than appended to. Parses only files that changed,
  shares each file's messages via `Arc` rather than re-parsing, drops entries for files no longer
  discovered, parses a path listed twice only once, and deliberately does **not** cache a parse
  failure so a half-written file is retried instead of remembered as empty.
  `IncrementalJsonlCache` stays the right choice for append-only JSONL, where only new bytes need
  parsing.

### Changed
- Qwen Code, Droid CLI, Cline and Kilo Code parse through `StampCache`. A reload now pays only for
  sessions that changed, where before each one re-parsed every source it had.
- `DroidCliAnalyzer::parse_session_file` is no longer `async`: it contained no awaits.

### Performance
- Real data, cold then warm parse of the same sources (release build):

  | analyzer | files | messages | cold | warm | speedup |
  | --- | --- | --- | --- | --- | --- |
  | Qwen Code | 81 | 5,861 | 21.5 ms | 2.7 ms | 7.9x |
  | Droid CLI | 64 | 3,498 | 25.8 ms | 1.0 ms | 26.6x |
  | Kilo Code | 112 | 3,164 | 30.6 ms | 1.2 ms | 24.6x |

  Totals identical across both passes for each analyzer.

### Still to do on BZL-74
- Kiro CLI (SQLite — needs a db+WAL stamp plus a row watermark, as OpenCode got in 0.35).
- Ollama, Cursor Agent, Antigravity and Warp (append-only JSONL — want cursor mode, not a stamp).
- Coalescing reload triggers so a file event and the Codex poll in one window cause one reload.
- Optionally persisting caches under `~/.config/splitrail/cache/` to cut the first draw.

### Note
- Cline has no data on this machine, so its wiring is mechanical but unverified against real files.

## [2.0.0-dash-0.80] - 2026-10-17

### Added
- **🏆 Records (`r` on the Summary tab).** Personal bests across every tool and machine, each
  with when it was set and where today stands against it (gold when you are on a record):
  - peak tk/s over 15 minutes and over an hour;
  - the biggest day by output and by $;
  - the longest run of days averaging the monthly goal (`[goal] monthly_tks`);
  - the best week and the best month;
  - the most sessions running at once, with the tools they were in. Ollama requests are left
    out, since each one is its own conversation.

  Records are kept in `~/.config/splitrail/records.json` and only ever go up, so they outlive
  the transcripts they came from. Beating one while the dashboard runs raises a macOS
  notification, "🏆 New record". They are worked out at most every 30 s, and only when the
  data changed.

## [2.0.0-dash-0.79] - 2026-10-17

BZL-136. Evaluating which billing rules the LiteLLM feed misses turned up two real mispricings and
ruled out two of the four upstream items as no-ops here.

### Fixed
- **1-hour cache writes were priced as 5-minute ones.** Anthropic bills a cache write by how long
  the entry lives: 1.25x base input for the 5-minute default, 2x for the 1-hour TTL. The feed
  publishes a single `cache_creation_input_token_cost` (the 5-minute rate), and every cache write
  was charged at it. Claude Code reports the split in `usage.cache_creation`, which was not parsed.
  Measured over the 30 most recent sessions: 655.7M of 658.1M cache-creation tokens (99.6%) used
  the 1h TTL, understating cache-write cost by **$2,617 (+59.6%)**. `Stats` gains
  `cache_creation_1h_tokens` (a subset of `cache_creation_tokens`, `#[serde(default)]`, so older
  records read 0), and `calculate_total_cost_with_cache_ttl` / `cache_cost_split` charge it
  correctly. The old signatures stay, delegating with a zero 1h count, so no other analyzer moves.
- **The Codex fallback model was hardcoded to `gpt-5`** (upstream #178). Sessions whose metadata
  never names a model are priced against the fallback; it now comes from Codex's own
  `config.toml` (honouring `CODEX_HOME`), falling back to `gpt-5` only when that is missing,
  empty, or malformed. On this machine the configured model is `gpt-6.1-sol`, affecting 3,423
  usage rows — worth $39.56 (+1%), small only because nearly all of that input is cached.

### Evaluated, not ported
- **Long-context surcharges** are already handled: `price_feed` parses
  `input_cost_per_token_above_{N}k_tokens` into `PriceTier` (BZL-62).
- **External pricing TOML** (#140) is already covered by `[pricing.overrides]`.
- **Priority/fast tiers** (#172, #260): the fields exist in the data (`usage.speed`,
  `usage.service_tier`) but are `"standard"` on all 55,391 assistant messages scanned, so there is
  nothing to price yet. Left alone rather than adding an unexercised code path.
- **Peak/off-peak** (#258) applies to providers not in use here (no DeepSeek-class model appears
  in any analyzer's data).

### Known issue
- `test_claude_code_analyzer_with_test_data` is flaky and fails on `55124a2` too: it compares the
  message count from two separate parses while the live corpus is being written, and its
  `HomeOverride` does not isolate discovery (it reads the real `~/.claude`). Needs its own ticket.

## [2.0.0-dash-0.78] - 2026-10-17

### Added
- **Kilo CLI column (BZL-133, upstream #118).** Kilo's OpenCode-based CLI keeps OpenCode's
  storage under `~/.local/share/kilo` (`kilo.db`, channel databases like `kilo-canary.db`, and
  legacy `storage/message` JSON). The OpenCode analyzer now takes a flavor (directory, database
  name, application, hash prefix), and `Kilo CLI` is a second instance of it, so both get the
  incremental SQLite reader. OpenCode also picks up `opencode-<channel>.db` databases. The old
  `@kilocode/cli` (0.6, `~/.kilocode`) stays in the Kilo Code column; Kilo CLI appears once it is
  upgraded. The 200 requests/hour free-model limit row is not included: plan limits have no
  hourly window yet.

## [2.0.0-dash-0.77] - 2026-10-17

### Fixed
- **Claude's weekly figure no longer flips between machines (70% / 79%).** A shared login's
  limits came from whichever machine's snapshot was written last, and an idle session keeps
  re-reporting the figure from its last API call, so devbox-a's stale 70% kept replacing this
  Mac's 79%. Snapshots for the same login are now merged per window: a later window wins, and
  within one window the higher figure (usage only grows until the reset).

### Changed
- **"Where to spend next" is a table you can scan.** One row per tool: a verb (USE NOW, USE,
  EASE OFF, WAIT, ON TRACK), what would be wasted (% and $), the tk/s needed against now, the
  reset, and a short why ("on pace for 81%", "runs out in 1d4h"). The full sentences stay in
  `quotas` / the MCP answer.

## [2.0.0-dash-0.76] - 2026-10-17

### Added
- **Quotas: what is left, and the pace needed to use it.** Each plan window now carries its
  spend so far (input, output, cache and API-equivalent cost since the window began, from the
  daily totals, every machine on the account summed), which measures what 1% of the allowance is
  worth. From that the Quotas view (`p`) shows **Left** (output tokens and dollars until the
  reset), **Need** (output tk/s from now to the reset to use it all; green when the current rate
  keeps up, yellow when behind), **Now** (the last 15 minutes) and **Locked out** (time between
  running out and the reset at the current pace). For Claude the ranking also gives the
  remainder as Opus 5.5 / Fable 5.1 / Sonnet 5.5 output tokens, repricing the window's own
  token mix on each model, so Fable's higher price is counted (about 1.8× Opus for a
  cache-heavy mix). Rows are ordered by how much is about to be wasted, as the ranking, and the
  columns fit narrower windows. `splitrail quotas`, its JSON and the MCP answers carry the same.

## [2.0.0-dash-0.75] - 2026-10-17

### Changed
- **Quotas view: one row per account, managed to the week or month.** Each row is the account's
  real allowance (its longest window), and the 5-hour window moves to a side column, `5h now`,
  with its use and when it refills: grey normally, yellow from 80% (switch soon), red from 95%
  (switch now). The ranking already judged plans by week or month and used the 5-hour window
  only to mark an account unavailable. `splitrail quotas` matches, the JSON gains `plans`, and
  `status.json` now carries each tool's account.

## [2.0.0-dash-0.74] - 2026-10-17

### Changed
- **Quotas view: an Account column and room to breathe.** Each row shows the account the tool is
  signed in to (the email with its organisation for Claude, Warp and Antigravity; the plan where
  that is all a tool reports, `chatgpt·plus`, `Kiro Free`), also in `splitrail quotas`, the MCP
  answers and the ranking. Columns sharing a login read `Claude (main), dev-a` rather than
  `Claude (main) = Claude@dev-a`. A blank line now sits under the tabs and under the header, with
  wider column spacing.

## [2.0.0-dash-0.73] - 2026-10-17

### Added
- **Quotas in the dashboard: press `p` on the Summary tab.** The same answers as
  `splitrail quotas`, live inside the dashboard: every plan window per tool and account (devbox
  columns merged with the login they share), used %, pace to the reset, unused share, when a
  fast pace runs out (red), and the reset as a countdown and a time (a date beyond this week),
  then where to spend next. `p` again returns to the summary.

## [2.0.0-dash-0.72] - 2026-10-17

### Added
- **Quota advisor: `splitrail-dashboard quotas` and an MCP server** (BZL-137). `quotas` lists
  every plan window on every machine (5h, week, month for Claude per account, Codex, Droid,
  Antigravity, Kiro, Cursor, Warp), with percent used, the exact reset time, the percent it is
  on pace for by the reset, the share that would go unused, and when a fast pace runs it out
  early, then ranks where to spend next: each plan's week or month by the share projected to
  expire unused per day until its reset, with nearly full ones marked unavailable until their
  window resets. Columns signed in to the same account (a devbox on the Mac's Claude login) are
  merged. `--json` prints it for scripts. `splitrail-dashboard mcp` serves the same as MCP tools
  (`quotas`, `where_to_spend`, `usage`) on stdio; register it with
  `claude mcp add splitrail -- splitrail-dashboard mcp`. Both read the running dashboard's
  `status.json`, so answers are instant and include the devboxes.

## [2.0.0-dash-0.71] - 2026-10-17

### Fixed
- **`CODEX_HOME` and `CLAUDE_CONFIG_DIR` are honoured** (BZL-134, upstream #224). Codex sessions
  are read from `$CODEX_HOME` when it is set, as Codex itself does, instead of always
  `~/.codex`; a Claude Code config directory named by `CLAUDE_CONFIG_DIR` is added as an account
  when it is not one already detected or configured. The other two upstream fixes in BZL-134
  (sub-cent cost rounding, new sessions in the live view) are bugs in structures upstream added
  after the fork; costs here are exact `f64` and new session files are picked up by the
  recursive watcher.

## [2.0.0-dash-0.70] - 2026-10-17

### Fixed
- **Claude Code no longer double-counts replies written as several records** (BZL-132, upstream
  #222). Records sharing a request and message id are cumulative snapshots of one billed request
  (input and cache repeat, output only grows), but rows with differing counts were added
  together. Tokens are now the largest seen per field, in both the batch dedup and the live fold;
  activity counted from each row's own content still adds. Across both accounts' history that
  removes 6,539 replies' worth of double counting: 898M cache-read and 29M cache-write tokens,
  roughly $600 at Opus rates. Output was barely affected (33k tokens), so tk/s is unchanged.

## [2.0.0-dash-0.69] - 2026-10-17

### Security
- **Dependency advisories cleared** (BZL-131). rustls 0.23.29 -> 0.23.46 (RUSTSEC-2026-0285,
  0346-0349) with rustls-webpki 0.103.15; quinn-proto 0.11.12 -> 0.11.19 (six advisories,
  including remote panics and memory exhaustion); bytes 1.12.1 (RUSTSEC-2026-0007);
  crossbeam-epoch 0.9.21 (RUSTSEC-2026-0204); anyhow 1.0.104 (unsound `downcast_mut`).
  `cargo audit` now reports no vulnerabilities; what remains is `paste` (unmaintained) and
  `lru` (unsound) through ratatui 0.29, which need a ratatui upgrade.

## [2.0.0-dash-0.68] - 2026-10-16

### Fixed
- **Custom and fine-tuned Ollama models get a street price instead of $0.** A model named for
  itself (`tev1:0.8b-mxfp8`, a decision model) matched no hosted model, so it cost $0 with a
  warning. The proxy now also logs each model's base family from Ollama (`qwen3_5`), and a model
  with no price of its own is priced as the cheapest hosted size of its family at or above its
  own size: `tev1` (0.75B) at Qwen 3.5 9B's median, $0.135 / $0.20 per 1M. Earlier lines borrow
  the family from later ones.

## [2.0.0-dash-0.67] - 2026-10-16

### Fixed
- **Devboxes update whenever the version changes.** A publisher counted as behind only when its
  publishing code differed, so a release that changed only the dashboard or a Mac-side script
  (0.66) left the devboxes on the old version label and `U` skipped them. A different version now
  counts as behind too, so every machine ends on the dashboard's version.
- **Decision-model transcripts in the Ollama activity** (BZL-129). `/v1/systemone` calls were
  logged with empty previews: the request carries `state` and `questions` rather than messages,
  and the reply carries `answers` rather than text. The prompt preview is now the state plus each
  question's instructions, the reply preview each answer with its confidence
  (`sky: blue (100%)`), checked against a live call.

## [2.0.0-dash-0.66] - 2026-10-16

### Fixed
- **Ollama decision-model calls (`/v1/systemone`) are logged** (BZL-129). The usage proxy only
  logged `/api/generate`, `/api/chat`, `/v1/chat/completions` and `/v1/completions`, so decision
  models (Ollama 0.35+) passed through unrecorded. Their `usage.input_tokens` / `output_tokens`
  and top-level `prompt_eval_cached_count` are now read, with the request's model when the reply
  names none; timing and energy apply as for the other endpoints.

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
