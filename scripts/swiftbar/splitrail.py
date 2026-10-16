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


# The title is drawn as a small image, a `tk/s` label over the number at a fixed width like the
# menu bar's other meters, by a Swift helper compiled once into the cache and kept running.
HERE = os.path.dirname(os.path.realpath(__file__))
RENDER_SOURCE = os.path.join(HERE, "render_title.swift")
RENDER_BINARY = os.path.expanduser("~/Library/Caches/splitrail/render_title")
_renderer = None
_rendered = {}


def renderer():
    global _renderer
    if _renderer and _renderer.poll() is None:
        return _renderer
    try:
        if not os.path.exists(RENDER_BINARY) or os.path.getmtime(RENDER_BINARY) < os.path.getmtime(RENDER_SOURCE):
            os.makedirs(os.path.dirname(RENDER_BINARY), exist_ok=True)
            subprocess.run(["/usr/bin/swiftc", "-O", RENDER_SOURCE, "-o", RENDER_BINARY],
                           check=True, capture_output=True, timeout=300)
        _renderer = subprocess.Popen([RENDER_BINARY], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
    except (OSError, subprocess.SubprocessError):
        _renderer = None
    return _renderer


def title(value, stale):
    """The menu bar line: the drawn label-over-number, or plain text if it cannot be drawn."""
    key = (value, stale)
    if key not in _rendered:
        image = ""
        proc = renderer()
        if proc:
            try:
                proc.stdin.write(f"tk/s\t{value}\t{0.35 if stale else 1}\n")
                proc.stdin.flush()
                image = proc.stdout.readline().strip()
            except OSError:
                image = ""
        if len(_rendered) > 500:
            _rendered.clear()
        _rendered[key] = image
    image = _rendered[key]
    if image:
        return f" | templateImage={image}"
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
    """The whole menu as lines: the title first, then the dropdown."""
    out = []
    print = out.append  # noqa: A001 - the menu is built line by line
    open_item = f"Open dashboard | bash={OPEN} terminal=false"
    try:
        with open(STATUS) as f:
            status = json.load(f)
    except (OSError, ValueError):
        print(title("—", True))
        print("---")
        print("Splitrail dashboard not running")
        print(open_item)
        return out
    age = time.time() - status.get("updated", 0)
    if age > STALE_SECS:
        print(title(short_rate(status.get('tks', 0)), True))
        print("---")
        print(f"Not updated for {countdown(int(age))} (dashboard stopped?) | color=gray")
    else:
        print(title(short_rate(status.get('tks', 0)), False))
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
    return out


def main():
    last = None
    while True:
        lines = menu()
        if lines != last:
            if last is not None:
                sys.stdout.write("~~~\n")
            sys.stdout.write("\n".join(lines) + "\n")
            sys.stdout.flush()
            last = lines
        time.sleep(1)


if __name__ == "__main__":
    main()
