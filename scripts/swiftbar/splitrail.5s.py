#!/usr/bin/python3
# <xbar.title>Splitrail</xbar.title>
# <xbar.desc>Live AI coding tk/s, today's cost and plan limits from the Splitrail dashboard.</xbar.desc>
# <swiftbar.hideRunInTerminal>true</swiftbar.hideRunInTerminal>
# <swiftbar.hideLastUpdated>true</swiftbar.hideLastUpdated>
"""SwiftBar plugin: reads ~/.config/splitrail/status.json, which the dashboard (or
`splitrail-dashboard mqtt` on this Mac) rewrites every few seconds, and refreshes every 5 s.

Install: ln -s "$PWD/scripts/swiftbar/splitrail.5s.py" ~/.swiftbar/
"""
import json
import os
import time

STATUS = os.path.expanduser("~/.config/splitrail/status.json")
STALE_SECS = 60
OPEN = os.path.join(os.path.dirname(os.path.realpath(__file__)), "open_dashboard.sh")
MONO = "font=Menlo size=12"
WINDOWS = [("five_hour", "5h"), ("seven_day", "week"), ("monthly", "month")]


def rate(tks):
    return f"{tks:.0f}" if tks >= 100 else f"{tks:.1f}"


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


def main():
    open_item = f"Open dashboard | bash={OPEN} terminal=false"
    try:
        with open(STATUS) as f:
            status = json.load(f)
    except (OSError, ValueError):
        print("⚡ — | color=gray")
        print("---")
        print("Splitrail dashboard not running")
        print(open_item)
        return
    age = time.time() - status.get("updated", 0)
    if age > STALE_SECS:
        print(f"⚡ {rate(status.get('tks', 0))} tk/s | color=gray")
        print("---")
        print(f"Not updated for {countdown(int(age))} (dashboard stopped?) | color=gray")
    else:
        print(f"⚡ {rate(status.get('tks', 0))} tk/s")
        print("---")
    print(f"Today ${status.get('cost_today', 0):,.2f} · {rate(status.get('tks', 0))} tk/s | {MONO}")
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


if __name__ == "__main__":
    main()
