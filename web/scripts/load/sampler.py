#!/usr/bin/env python3
"""Samples the memory and CPU time of Gelabber's processes, one JSON line per
interval, until stopped.

    python3 scripts/load/sampler.py samples.jsonl [interval_seconds]

Per group it records RSS, PSS and USS (private) in KiB from
/proc/<pid>/smaps_rollup, the process count and the summed CPU ticks.
Groups: api (gelabber-api), media (gelabber-media, mediasoup runs inside it),
postgres, redis, desktop (the native app, `gelabber`), and one group per
browser that the load scenarios label (``chrome:<label>``: its whole process
tree, with each process's type and PSS under ``parts``). The phase label
comes from the file the scenarios write (phases.mjs). Linux only; reading
other users' processes needs the same user or root.
"""
import json
import os
import sys
import tempfile
import time

OUT = sys.argv[1]
INTERVAL = float(sys.argv[2]) if len(sys.argv) > 2 else 1.0
TMP = tempfile.gettempdir()
PHASE_FILE = os.environ.get("GELABBER_LOAD_PHASE_FILE", os.path.join(TMP, "gelabber-load-phase"))
LABEL_FILE = os.environ.get("GELABBER_LOAD_LABEL_FILE", os.path.join(TMP, "gelabber-load-labels.json"))
GROUPS = {"gelabber-api": "api", "gelabber-media": "media", "postgres": "postgres",
          "redis-server": "redis", "gelabber": "desktop"}


def read(path, mode="r"):
    try:
        with open(path, mode) as f:
            return f.read()
    except OSError:
        return None


def stat(pid):
    """(parent pid, utime + stime ticks) or None."""
    s = read(f"/proc/{pid}/stat")
    if not s:
        return None
    fields = s[s.rfind(")") + 2:].split()
    return int(fields[1]), int(fields[11]) + int(fields[12])


def memory(pid):
    s = read(f"/proc/{pid}/smaps_rollup")
    if not s:
        return None
    m = {}
    for line in s.splitlines()[1:]:
        key, value = line.split(":", 1)
        m[key] = int(value.split()[0])
    return {"Rss": m.get("Rss", 0), "Pss": m.get("Pss", 0),
            "Uss": m.get("Private_Clean", 0) + m.get("Private_Dirty", 0)}


def process_type(pid):
    cmd = (read(f"/proc/{pid}/cmdline", "rb") or b"").replace(b"\0", b" ").decode(errors="replace")
    return cmd.split("--type=")[1].split()[0] if "--type=" in cmd else "browser"


def groups():
    procs = {}
    for name in os.listdir("/proc"):
        if name.isdigit():
            st = stat(int(name))
            if st:
                procs[int(name)] = ((read(f"/proc/{name}/comm") or "").strip(), st[0])
    found = {}
    for pid, (comm, _) in procs.items():
        if comm in GROUPS:
            found.setdefault(GROUPS[comm], []).append(pid)
    try:
        labels = json.loads(read(LABEL_FILE) or "{}")
    except ValueError:
        labels = {}
    children = {}
    for pid, (_, parent) in procs.items():
        children.setdefault(parent, []).append(pid)
    for root, label in labels.items():
        stack = [int(root)] if int(root) in procs else []
        while stack:
            pid = stack.pop()
            found.setdefault("chrome:" + label, []).append(pid)
            stack.extend(children.get(pid, []))
    return found


def main():
    with open(OUT, "a") as out:
        while True:
            start = time.time()
            record = {"t": round(start, 2), "phase": (read(PHASE_FILE) or "").strip()}
            for group, pids in groups().items():
                total = {"Rss": 0, "Pss": 0, "Uss": 0, "n": 0, "cpu": 0}
                parts = []
                for pid in pids:
                    m = memory(pid)
                    st = stat(pid)
                    if not m or not st:
                        continue
                    for key in ("Rss", "Pss", "Uss"):
                        total[key] += m[key]
                    total["n"] += 1
                    total["cpu"] += st[1]
                    if group.startswith("chrome:"):
                        parts.append([process_type(pid), m["Pss"]])
                if group.startswith("chrome:"):
                    total["parts"] = parts
                record[group] = total
            meminfo = {}
            for line in (read("/proc/meminfo") or "").splitlines():
                key, value = line.split(":", 1)
                meminfo[key] = int(value.split()[0])
            if "MemTotal" in meminfo:
                record["sys_used"] = meminfo["MemTotal"] - meminfo["MemAvailable"]
            record["load1"] = os.getloadavg()[0]
            out.write(json.dumps(record) + "\n")
            out.flush()
            time.sleep(max(0.0, INTERVAL - (time.time() - start)))


if __name__ == "__main__":
    main()
