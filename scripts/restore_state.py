"""Merge an old Mac's ~/.config/splitrail into this one without losing anything newer.
Usage: python3 -I scripts/restore_state.py OLD_DIR NEW_DIR [--dry-run]

See docs/RESTORE.md. Stop the dashboard first: it rewrites these files from memory."""
import json, os, shutil, sys
old, new = sys.argv[1], sys.argv[2]
dry = "--dry-run" in sys.argv
SKIP = {"cursor_api_key", "factory_api_key", "mqtt_password", "warp_token",  # secrets: set on the new Mac
        "prices.json", "status.json", "status.lock", "update.log", "update_durations.json"}
done = []

def load(p):
    with open(p) as f: return json.load(f)

def write(p, data):
    if dry: return
    os.makedirs(os.path.dirname(p), exist_ok=True)
    tmp = p + ".tmp"
    with open(tmp, "w") as f: json.dump(data, f)
    os.replace(tmp, p)

def merge_closes(o, n):
    key = lambda c: (c.get("tool"), c.get("account"), c.get("window"), c.get("resets_at"))
    out = {key(c): c for c in o}
    for c in n:
        k = key(c)
        if k in out:
            best = dict(out[k]); best["peak"] = max(out[k].get("peak", 0), c.get("peak", 0))
            best["announced"] = out[k].get("announced", False) or c.get("announced", False)
            out[k] = best
        else:
            out[k] = c
    return sorted(out.values(), key=lambda c: c.get("resets_at") or 0)

def merge_history(o, n):
    out = dict(o)
    for k, entries in n.items():
        seen = {json.dumps(e, sort_keys=True) for e in out.get(k, [])}
        out[k] = out.get(k, []) + [e for e in entries if json.dumps(e, sort_keys=True) not in seen]
        out[k].sort(key=lambda e: e.get("since", 0))
    return out

def merge_records(o, n):
    out = dict(o)
    for k, v in n.items():
        if k not in out or (isinstance(v, dict) and isinstance(out[k], dict)
                            and (v.get("value") or 0) > (out[k].get("value") or 0)):
            out[k] = v
    return out

def merge_times(o, n):
    out = dict(n); lines = dict(n.get("lines", {}))
    for k, v in o.get("lines", {}).items():
        mine = lines.get(k, [])
        lines[k] = v + mine[len(v):]  # old stamps win for lines both saw
    out["lines"] = lines
    return out

SPECIAL = {"plan_closes.json": merge_closes, "plan_history.json": merge_history,
           "records.json": merge_records, "cursor/transcript_times.json": merge_times}

for root, dirs, files in os.walk(old):
    for name in files:
        src = os.path.join(root, name)
        rel = os.path.relpath(src, old)
        if name in SKIP or name.endswith((".tmp", ".bak-doubled", ".bak-misfiled")):
            continue
        dst = os.path.join(new, rel)
        if not os.path.exists(dst):
            done.append(("copy", rel))
            if not dry:
                os.makedirs(os.path.dirname(dst), exist_ok=True); shutil.copy2(src, dst)
        elif rel in SPECIAL:
            done.append(("merge", rel))
            write(dst, SPECIAL[rel](load(src), load(dst)))
        elif name.endswith(".jsonl"):
            with open(src) as f: o = f.read().splitlines()
            with open(dst) as f: n = f.read().splitlines()
            have = set(o)
            extra = [l for l in n if l not in have]
            if len(o) + len(extra) != len(n):
                done.append(("merge-lines", f"{rel} (+{len(o) + len(extra) - len(n)} old lines)"))
                if not dry:
                    with open(dst + ".tmp", "w") as f: f.write("\n".join(o + extra) + "\n")
                    os.replace(dst + ".tmp", dst)
        # Other files present on both: the new Mac's copy is newer; keep it.

for action, rel in done:
    print(f"{action:12} {rel}")
print(f"{len(done)} changes{' (dry run)' if dry else ''}")
