#!/usr/bin/python3
# <xbar.title>Splitrail</xbar.title>
# <xbar.desc>Live AI coding tk/s, today's cost and plan limits from the Splitrail dashboard.</xbar.desc>
# <swiftbar.type>streamable</swiftbar.type>
# <swiftbar.hideRunInTerminal>true</swiftbar.hideRunInTerminal>
# <swiftbar.hideLastUpdated>true</swiftbar.hideLastUpdated>
"""SwiftBar plugin: reads ~/.config/splitrail/status.json, which the dashboard (or
`splitrail-dashboard mqtt` on this Mac) rewrites about every second.

Streamable: one long-running process that rereads the small file each second and prints a new
menu only when something changed, instead of SwiftBar starting Python every refresh (about
30 ms of CPU each time).

Install: ln -s "$PWD/scripts/swiftbar/splitrail.py" ~/.swiftbar/
"""
import json
import os
import subprocess
import sys
import time

STATUS = os.path.expanduser("~/.config/splitrail/status.json")
STALE_SECS = 60
OPEN = os.path.join(os.path.dirname(os.path.realpath(__file__)), "open_dashboard.sh")
MONO = "font=Menlo size=12"
WINDOWS = [("five_hour", "5h"), ("seven_day", "week"), ("monthly", "month")]


def rate(tks):
    return f"{tks:.0f}" if tks >= 100 else f"{tks:.1f}"


def short_rate(tks):
    """The menu bar title's number: at most four characters, to keep it narrow beside the notch."""
    if tks >= 1000:
        return f"{tks / 1000:.1f}k"
    return f"{tks:.0f}" if tks >= 10 else f"{tks:.1f}"


# The menu bar item itself is a small native helper (`menubar.swift`, compiled once into the
# cache and kept running): only a status item that draws at display time gets each display's
# menu bar colour. This plugin works out the title and menu and streams them to it, printing
# nothing to SwiftBar, which then shows no item of its own. Without the helper (no Swift
# compiler), SwiftBar shows a plain text title and the same menu.
HERE = os.path.dirname(os.path.realpath(__file__))
HELPER_SOURCE = os.path.join(HERE, "menubar.swift")
HELPER_BINARY = os.path.expanduser("~/Library/Caches/splitrail/menubar")
_helper = None


def helper():
    global _helper
    if _helper and _helper.poll() is None:
        return _helper
    try:
        if not os.path.exists(HELPER_BINARY) or os.path.getmtime(HELPER_BINARY) < os.path.getmtime(HELPER_SOURCE):
            os.makedirs(os.path.dirname(HELPER_BINARY), exist_ok=True)
            subprocess.run(["/usr/bin/swiftc", "-O", HELPER_SOURCE, "-o", HELPER_BINARY],
                           check=True, capture_output=True, timeout=300)
        _helper = subprocess.Popen([HELPER_BINARY], stdin=subprocess.PIPE, text=True)
    except (OSError, subprocess.SubprocessError):
        _helper = None
    return _helper


GREEN, AMBER, RED = "#30c552", "#ff9f0a", "#ff453a"


def pace_dot(status):
    """The dot beside `tk/s`: (colour, filled), or None without a monthly goal.

    Colour: the current speed against what the rest of the month needs to finish on the goal
    (green at or above it, amber at half or more, red below). Filled when the month's average so
    far is at or above the goal, a ring while it is behind.
    """
    month = status.get("month") or {}
    goal = month.get("goal_tks") or 0
    if goal <= 0:
        return None
    needed = month.get("needed_tks", goal)
    tks = status.get("tks", 0)
    colour = GREEN if tks >= needed else AMBER if tks >= needed / 2 else RED
    return colour, month.get("avg_tks", 0) >= goal


def helper_title(value, stale, dot=None):
    """The helper's title line."""
    dot_colour, filled = dot if dot else ("", False)
    return f"TITLE\ttk/s\t{value}\t{0.5 if stale else 1}\t{dot_colour}\t{1 if filled else 0}"


def text_title(value, stale):
    """SwiftBar's own title, when the helper cannot run."""
    return f"tk/s {value} | font=Menlo size=11" + (" color=gray" if stale else "")


def countdown(secs):
    if secs is None:
        return ""
    if secs <= 0:
        return "now"
    minutes = secs // 60
    days, hours, mins = minutes // 1440, (minutes % 1440) // 60, minutes % 60
    if days:
        return f"{days}d{hours}h"
    return f"{hours}h{mins:02d}m" if hours else f"{mins}m"


def color(used):
    return "#e5534b" if used >= 90 else "#d29922" if used >= 70 else "#57ab5a"


def menu():
    """The title as (value, stale, dot), and the dropdown's lines."""
    out = []
    print = out.append  # noqa: A001 - the menu is built line by line
    open_item = f"Open dashboard | bash={OPEN} terminal=false"
    try:
        with open(STATUS) as f:
            status = json.load(f)
    except (OSError, ValueError):
        print("Splitrail dashboard not running")
        print(open_item)
        return ("—", True, None), out
    age = time.time() - status.get("updated", 0)
    stale = age > STALE_SECS
    head = (short_rate(status.get("tks", 0)), stale, None if stale else pace_dot(status))
    if stale:
        print(f"Not updated for {countdown(int(age))} (dashboard stopped?) | color=gray")
    print(f"Today ${status.get('cost_today', 0):,.2f} · {rate(status.get('tks', 0))} tk/s | {MONO}")
    month = status.get("month") or {}
    dot = pace_dot(status)
    if dot:
        goal, avg, needed = month["goal_tks"], month.get("avg_tks", 0), month.get("needed_tks", 0)
        mark = "●" if dot[1] else "○"
        state = "on track" if dot[1] else f"{avg / goal:.0%} of goal"
        print(f"{mark} Month avg {rate(avg)} tk/s of {rate(goal)} · {state} | color={dot[0]} {MONO}")
        print(f"--Rest of month needs {rate(needed)} tk/s (now {rate(status.get('tks', 0))}) | {MONO}")
        print(f"--{month.get('output_tokens', 0) / 1e6:,.1f}M output tokens this month | {MONO}")
        print(f"--Dot colour: now vs needed · green ≥ needed, amber ≥ half, red below | {MONO}")
        print(f"--Filled: month average at or above goal · ring: behind | {MONO}")
    print("---")
    for tool in status.get("tools", []):
        name = tool["name"]
        print(f"{name[:22]:<22} {rate(tool['tks']):>6} tk/s  ${tool['cost_today']:>7,.2f} | {MONO}")
        for key, label in WINDOWS:
            window = (tool.get("limits") or {}).get(key)
            if window and window.get("used") is not None:
                resets = countdown(window.get("resets_in"))
                tail = f" · resets {resets}" if resets else ""
                print(f"--{label} {window['used']:.0f}%{tail} | color={color(window['used'])} {MONO}")
    publishers = status.get("publishers", [])
    if publishers:
        print("---")
        for p in publishers:
            version = (p.get("version") or "old").rsplit("-", 1)[-1]
            state = "offline" if not p.get("online") else "update with U" if p.get("behind") else "current"
            print(f"{p['host']} {version} · {state} | color={'gray' if state == 'offline' else '#d29922' if p.get('behind') else '#57ab5a'}")
    print("---")
    print(open_item)
    print(f"splitrail {status.get('version', '')} | color=gray size=11")
    return head, out


def main():
    last, printed = None, True  # A "~~~" first clears what SwiftBar showed before.
    while True:
        (value, stale, dot), lines = menu()
        proc = helper()
        if proc:
            block = [helper_title(value, stale, dot), *lines]
            if block != last:
                try:
                    proc.stdin.write("\n".join(block) + "\n~~~\n")
                    proc.stdin.flush()
                    last = block
                except OSError:
                    last = None  # Restarted next time round.
            if printed:
                # Back from the fallback: clear SwiftBar's own item.
                sys.stdout.write("~~~\n")
                sys.stdout.flush()
                printed = False
        else:
            block = [text_title(value, stale), "---", *lines]
            if block != last:
                if printed:
                    sys.stdout.write("~~~\n")
                sys.stdout.write("\n".join(block) + "\n")
                sys.stdout.flush()
                last, printed = block, True
        time.sleep(1)


if __name__ == "__main__":
    main()
