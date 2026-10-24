#!/usr/bin/env python3
"""Claude Code status line that records plan-limit usage for Splitrail (BZL-66).

Claude Code runs the `statusLine` command with a JSON payload on stdin and shows whatever it
prints. That payload carries `rate_limits` (5-hour and weekly usage with reset times), which
appears nowhere in the transcripts. This script saves the latest snapshot per account to
`~/.config/splitrail/limits/<label>.json` for the dashboard, and prints a one-line summary.

Configure it once per account, passing that account's label:

    "statusLine": {
      "type": "command",
      "command": "python3 /path/to/splitrail/scripts/claude_statusline_capture.py main"
    }

It never fails the status line: any error still prints something and exits 0.

Every open session on an account runs its own status line, and each reports the limits it saw
on its own last API call, so an idle session reports old numbers. Overwriting the file with
whatever arrived last made the dashboard flicker (weekly 5% -> 24% -> 9% within a minute).
Snapshots are therefore merged per window: usage within a window only grows, so for the same
window the higher figure wins, a later reset time means a new window, and an earlier reset time
is a stale session and is ignored.

One config directory can also be logged in to different Claude accounts over time (`/login`
switches the whole directory), and each has its own windows. The payload does not say which
account it is, so the login is read from the directory's `.claude.json` and limits are kept per
login; the top-level `rate_limits` is the current login's. A session that has not made an API
call since the switch still reports the previous account's numbers, so a report identical to
what that session reported under the previous login is not attributed to anyone.
"""

import fcntl
import json
import os
import sys
import tempfile
import time
from datetime import datetime

LIMITS_DIR = os.path.expanduser("~/.config/splitrail/limits")
# Which login each session ran under, from when (BZL-283): one line per change, kept for good, so
# the dashboard can credit each reply's tokens to the allowance they came out of.
LOGINS_DIR = os.path.expanduser("~/.config/splitrail/logins")
# Claude Code's own per-session totals, to check splitrail's sums against (BZL-193).
SESSIONS_DIR = os.path.expanduser("~/.config/splitrail/claude_sessions")
# Ledger entries not updated in this long are dropped.
LEDGER_TTL_SECS = 45 * 86400

# Windows the payload may carry, in display order, with their short labels.
WINDOWS = [("five_hour", "5h"), ("seven_day", "7d")]


def parse_reset(value):
    """`resets_at` as epoch seconds, from either epoch seconds/ms or an ISO-8601 string."""
    if value is None:
        return None
    if isinstance(value, (int, float)):
        return value / 1000 if value > 1e12 else float(value)
    try:
        return datetime.fromisoformat(str(value).replace("Z", "+00:00")).timestamp()
    except ValueError:
        return None


def used_percent(window):
    """Usage as 0-100, from `used_percentage` or a 0-1 / 0-100 `utilization`."""
    for key in ("used_percentage", "utilization"):
        value = window.get(key)
        if isinstance(value, (int, float)):
            return value * 100 if key == "utilization" and value <= 1 else float(value)
    return None


def countdown(seconds):
    if seconds <= 0:
        return "now"
    minutes = int(seconds // 60)
    days, minutes = divmod(minutes, 24 * 60)
    hours, minutes = divmod(minutes, 60)
    if days:
        return f"{days}d{hours}h"
    if hours:
        return f"{hours}h{minutes:02d}m"
    return f"{minutes}m"


# Reset times of the same window can differ slightly between reports; beyond this they are
# different windows.
SAME_WINDOW_TOLERANCE_SECS = 30 * 60


def merge_window(old, new):
    """The more current of two reports of one window (dicts with used_percentage/resets_at)."""
    if not isinstance(new, dict) or used_percent(new) is None:
        return old
    if not isinstance(old, dict) or used_percent(old) is None:
        return new
    old_reset, new_reset = parse_reset(old.get("resets_at")), parse_reset(new.get("resets_at"))
    if old_reset is not None and new_reset is not None:
        if new_reset > old_reset + SAME_WINDOW_TOLERANCE_SECS:
            return new  # A newer window has started.
        if new_reset < old_reset - SAME_WINDOW_TOLERANCE_SECS:
            return old  # A stale session still reporting the previous window.
    return new if used_percent(new) >= used_percent(old) else old


def merge_limits(old, new):
    old = old if isinstance(old, dict) else {}
    new = new if isinstance(new, dict) else {}
    merged = {}
    for key in set(old) | set(new):
        window = merge_window(old.get(key), new.get(key))
        if window is not None:
            merged[key] = window
    return merged or None


def replace_limits(old, new):
    """`old` with each window `new` reports replaced by the new figure."""
    merged = dict(old) if isinstance(old, dict) else {}
    for key, window in (new if isinstance(new, dict) else {}).items():
        if isinstance(window, dict) and used_percent(window) is not None:
            merged[key] = window
    return merged or None


def fill_limits(old, new):
    """`old` plus any window only `new` reports."""
    return replace_limits(new, old)


# Temp files older than this were left by a status line process killed mid-write (agy kills
# the previous run when the agent state changes quickly).
STALE_TMP_SECS = 60


def sweep_stale_tmp(directory, now=None):
    now = time.time() if now is None else now
    try:
        names = os.listdir(directory)
    except OSError:
        return
    for name in names:
        if name.startswith(".") and name.endswith(".tmp"):
            path = os.path.join(directory, name)
            try:
                if now - os.path.getmtime(path) > STALE_TMP_SECS:
                    os.remove(path)
            except OSError:
                pass


def claude_json_path(label, environ=os.environ, home=os.path.expanduser("~")):
    """The `.claude.json` of the config directory this status line runs for."""
    config_dir = environ.get("CLAUDE_CONFIG_DIR")
    if config_dir:
        return os.path.join(os.path.expanduser(config_dir), ".claude.json")
    if label != "main" and os.path.isdir(os.path.join(home, f".claude-{label}")):
        return os.path.join(home, f".claude-{label}", ".claude.json")
    return os.path.join(home, ".claude.json")


TEAM_ORG_TYPES = ("team", "enterprise")


def login_from_account(account):
    """(key, display name) for an `oauthAccount`. One email can belong to a personal plan and
    to a team, each with its own limits, so the organization is part of the key."""
    email = account.get("emailAddress") or account.get("accountUuid")
    if not email:
        return None, None
    name = email.split("@")[0]
    org_type = account.get("organizationType") or ""
    if any(t in org_type for t in TEAM_ORG_TYPES):
        org = account.get("organizationName") or account.get("organizationUuid") or "team"
        return f"{email} ({org})", f"{name}·team"
    return f"{email} (personal)", name


def current_login(label):
    """(key, display name) of the account the config directory is logged in to."""
    try:
        with open(claude_json_path(label)) as f:
            account = json.load(f).get("oauthAccount") or {}
    except (OSError, ValueError, AttributeError):
        return None, None
    return login_from_account(account)


def plan_from_account(account):
    """The plan behind an `oauthAccount`, for telling allowances apart and noticing an upgrade
    or downgrade: `{"label": "Max 20x", "type": "claude_max", "seat": None, "tier":
    "default_claude_max_20x", "extra_usage": True}`. None without an account."""
    if not isinstance(account, dict) or not account:
        return None
    kind = account.get("organizationType") or ""
    seat = account.get("seatTier") or None
    tier = account.get("organizationRateLimitTier") or account.get("userRateLimitTier") or ""
    if seat:
        label = {"team_standard": "Team Std", "team_premium": "Team Prem"}.get(seat, seat.replace("_", " ").title())
    elif "max_20x" in tier:
        label = "Max 20x"
    elif "max_5x" in tier:
        label = "Max 5x"
    else:
        label = {"claude_max": "Max", "claude_pro": "Pro", "claude_team": "Team",
                 "claude_enterprise": "Enterprise"}.get(kind, kind.replace("claude_", "").title() or "?")
    return {"label": label, "type": kind or None, "seat": seat, "tier": tier or None,
            "extra_usage": bool(account.get("hasExtraUsageEnabled"))}


def current_plan(label):
    try:
        with open(claude_json_path(label)) as f:
            return plan_from_account(json.load(f).get("oauthAccount") or {})
    except (OSError, ValueError, AttributeError):
        return None


# Sessions not heard from in this long are forgotten.
SESSION_TTL_SECS = 24 * 3600
# Logins not heard from in this long are dropped: even their weekly window has rolled over.
# Other logins are kept for "Where to spend next": a Team seat unused this week is a whole week
# of allowance waiting. Forgotten only after a year unused (an account that is gone).
LOGIN_TTL_SECS = 365 * 24 * 3600


def update_snapshot(previous, label, payload, login, now, login_name=None):
    """The new snapshot file contents, from the previous contents and one status line payload."""
    previous = previous if isinstance(previous, dict) else {}
    logins = previous.get("logins") if isinstance(previous.get("logins"), dict) else {}
    logins = {
        key: entry
        for key, entry in logins.items()
        if isinstance(entry, dict) and now - entry.get("captured_at", 0) < LOGIN_TTL_SECS
    }
    sessions = previous.get("sessions") if isinstance(previous.get("sessions"), dict) else {}
    sessions = {
        sid: seen
        for sid, seen in sessions.items()
        if isinstance(seen, dict) and now - seen.get("seen", 0) < SESSION_TTL_SECS
    }

    reported = payload.get("rate_limits")
    fingerprint = json.dumps(reported, sort_keys=True)
    session_id = payload.get("session_id")
    last = sessions.get(session_id) if session_id else None
    # Only a repeat across a login switch is suspect: the session has not called the API since,
    # so these are the previous account's numbers. A repeat under the same login is just
    # unchanged usage. Sessions recorded before logins were tracked count as the same login.
    switched = last is not None and last.get("login", login) != login
    fresh = reported is not None and not (switched and last.get("fp") == fingerprint)
    # A session whose numbers just changed made an API call under this login, so they are the
    # account's current state and replace what is stored. Merging them instead ("same window,
    # keep the higher") would let a wrong figure stick for the rest of the window, e.g. another
    # login's 100% written by a session that switched before it was ever seen. A session's first
    # report counts too, unless another login's sessions report exactly the same numbers; that
    # one is merged. A repeat carries nothing new (it was applied when it changed), so it only
    # fills windows nothing else has reported.
    other_logins_fps = {
        seen.get("fp")
        for sid, seen in sessions.items()
        if sid != session_id and seen.get("login", login) != login
    }
    repeat = last is not None and last.get("fp") == fingerprint
    if last is None:
        mode = "merge" if fingerprint in other_logins_fps else "replace"
    else:
        mode = "fill" if repeat else "replace"
    if session_id:
        sessions[session_id] = {"fp": fingerprint, "seen": now, "login": login}

    key = login or "unknown"
    entry = logins.get(key) if isinstance(logins.get(key), dict) else {}
    if fresh:
        entry = {
            "captured_at": now,
            "name": login_name or login_name_of(key),
            **({"plan": entry["plan"]} if isinstance(entry.get("plan"), dict) else {}),
            "rate_limits": (
                replace_limits(entry.get("rate_limits"), reported)
                if mode == "replace"
                else fill_limits(entry.get("rate_limits"), reported)
                if mode == "fill"
                else merge_limits(entry.get("rate_limits"), reported)
            ),
        }
        logins[key] = entry
    return {
        "account": label,
        "login": login,
        "login_name": login_name or (login_name_of(login) if login else None),
        "captured_at": entry.get("captured_at", now),
        "rate_limits": entry.get("rate_limits"),
        "logins": logins,
        "sessions": sessions,
        # Kept so a payload without rate_limits can be diagnosed from the file alone.
        "payload_keys": sorted(payload.keys()),
    }


def save_snapshot(label, payload, login=None):
    """Merge this payload into the account's snapshot and return the snapshot. `login` defaults
    to the Claude config directory's current login."""
    os.makedirs(LIMITS_DIR, exist_ok=True)
    path = os.path.join(LIMITS_DIR, f"{label}.json")
    if login:
        login_name = login.split("@")[0]
    else:
        login, login_name = current_login(label)
    # Sessions refresh concurrently; serialise the read-merge-write so none is lost.
    with open(os.path.join(LIMITS_DIR, f".{label}.lock"), "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        sweep_stale_tmp(LIMITS_DIR)
        try:
            with open(path) as f:
                previous = json.load(f)
        except (OSError, ValueError):
            previous = None
        now = time.time()
        if login_changed(previous, payload.get("session_id"), login):
            record_login(label, now, payload["session_id"], login)
        snapshot = update_snapshot(previous, label, payload, login, now, login_name)
        # The signed-in login's plan, kept on its entry so other logins' plans stay known.
        plan = current_plan(label) if not payload.get("_plan_test") else payload["_plan_test"]
        if plan and login and isinstance(snapshot.get("logins", {}).get(login), dict):
            snapshot["logins"][login]["plan"] = plan
            snapshot["plan"] = plan
        # Write-then-rename: the dashboard may read while a status line refresh is writing.
        fd, tmp = tempfile.mkstemp(dir=LIMITS_DIR, prefix=f".{label}.", suffix=".tmp")
        with os.fdopen(fd, "w") as f:
            json.dump(snapshot, f)
        os.replace(tmp, path)
    return snapshot


def login_changed(previous, session_id, login):
    """Whether this session is on a login the snapshot has not recorded for it yet: a new
    session, or a `/login` in a running one."""
    if not session_id or not login:
        return False
    sessions = (previous or {}).get("sessions") if isinstance(previous, dict) else None
    last = sessions.get(session_id) if isinstance(sessions, dict) else None
    return not isinstance(last, dict) or last.get("login") != login


def record_login(label, now, session_id, login, directory=None):
    """Append `{"t", "session", "login"}` to the account's login history."""
    directory = directory or LOGINS_DIR
    os.makedirs(directory, exist_ok=True)
    line = json.dumps({"t": round(now, 3), "session": session_id, "login": login}) + "\n"
    with open(os.path.join(directory, f"{label}.jsonl"), "a") as f:
        f.write(line)


def update_ledger(previous, payload, now):
    """The session ledger after one payload: Claude Code's running cost for the session, which
    it adds up itself from the API responses, so independent of splitrail's log parsing. Only
    `cost` is a running total; `context_window`'s token counts describe the current context and
    fall back after a turn, so they are not kept. The cost only grows within a process; a lower
    report (a restarted process resuming the session) starts a new segment."""
    ledger = previous if isinstance(previous, dict) else {}
    ledger = {
        sid: entry
        for sid, entry in ledger.items()
        if isinstance(entry, dict) and now - entry.get("last_seen", 0) < LEDGER_TTL_SECS
    }
    session_id = payload.get("session_id")
    cost = payload.get("cost") if isinstance(payload.get("cost"), dict) else {}
    totals = {"cost_usd": cost.get("total_cost_usd"), "api_duration_ms": cost.get("total_api_duration_ms")}
    if not session_id or totals["cost_usd"] is None:
        return ledger
    entry = ledger.get(session_id) or {
        "first_seen": now,
        "transcript_path": payload.get("transcript_path"),
        "segments": [],
    }
    segments = entry.setdefault("segments", [])
    current = segments[-1] if segments else None
    restarted = current is not None and totals["cost_usd"] < (current.get("cost_usd") or 0)
    if current is None or restarted:
        segments.append({"since": now, **totals})
    else:
        current.update(totals)
    entry["last_seen"] = now
    entry["model"] = (payload.get("model") or {}).get("id")
    ledger[session_id] = entry
    return ledger


def save_ledger(label, payload):
    os.makedirs(SESSIONS_DIR, exist_ok=True)
    path = os.path.join(SESSIONS_DIR, f"{label}.json")
    with open(os.path.join(SESSIONS_DIR, f".{label}.lock"), "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        try:
            with open(path) as f:
                previous = json.load(f)
        except (OSError, ValueError):
            previous = None
        ledger = update_ledger(previous, payload, time.time())
        fd, tmp = tempfile.mkstemp(dir=SESSIONS_DIR, prefix=f".{label}.", suffix=".tmp")
        with os.fdopen(fd, "w") as f:
            json.dump(ledger, f)
        os.replace(tmp, path)


def login_name_of(login):
    return login.split("@")[0] if login else None


def summary(label, payload, limits, snapshot=None):
    snapshot = snapshot or {}
    name = snapshot.get("login_name") or login_name_of(snapshot.get("login"))
    parts = [f"{label}:{name}" if name else label]
    model = (payload.get("model") or {}).get("display_name")
    if model:
        parts.append(model)
    limits = limits or {}
    now = time.time()
    for key, short in WINDOWS:
        window = limits.get(key)
        if not isinstance(window, dict):
            continue
        pct = used_percent(window)
        reset = parse_reset(window.get("resets_at"))
        text = short
        if pct is not None:
            text += f" {pct:.0f}%"
        if reset is not None:
            text += f" ↻{countdown(reset - now)}"
        parts.append(text)
    # The other logins' weekly usage, to pick which account to switch to.
    others = []
    for login, entry in sorted((snapshot.get("logins") or {}).items()):
        if login == snapshot.get("login") or not isinstance(entry, dict):
            continue
        week = (entry.get("rate_limits") or {}).get("seven_day")
        if not isinstance(week, dict) or used_percent(week) is None:
            continue
        reset = parse_reset(week.get("resets_at"))
        pct = 0 if reset is not None and reset <= now else used_percent(week)
        others.append(f"{entry.get('name') or login_name_of(login)} {pct:.0f}%")
    if others:
        parts.append("7d " + ", ".join(others))
    return " · ".join(parts)


def main():
    label = sys.argv[1] if len(sys.argv) > 1 else "main"
    try:
        payload = json.load(sys.stdin)
    except (json.JSONDecodeError, ValueError):
        print(label)
        return
    try:
        # Show the merged view, so every session agrees on the account's usage.
        snapshot = save_snapshot(label, payload)
        limits = snapshot["rate_limits"]
    except OSError:
        snapshot = None
        limits = payload.get("rate_limits")  # A read-only or full disk must not break the line.
    try:
        save_ledger(label, payload)
    except (OSError, ValueError, TypeError):
        pass
    print(summary(label, payload, limits, snapshot))


if __name__ == "__main__":
    try:
        main()
    except Exception:  # noqa: BLE001 - the status line must always render
        print(sys.argv[1] if len(sys.argv) > 1 else "main")
