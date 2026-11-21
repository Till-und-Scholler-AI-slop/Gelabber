#!/usr/bin/env python3
"""Averages sampler.py output per phase.

    python3 scripts/load/summarize.py samples.jsonl [group ...]

Prints one row per phase (in order of appearance): samples, then per group
the average RSS and PSS in MB and the CPU use in cores over the phase.
Default groups: api media postgres redis. A phase label that repeats later
(a second run with the same names) is reported as a separate row.
"""
import json
import sys


def phases(path):
    rows, current = [], None
    with open(path) as f:
        for line in f:
            record = json.loads(line)
            if current is None or record["phase"] != current[0]:
                current = (record["phase"], [])
                rows.append(current)
            current[1].append(record)
    return rows


def summary(records, group, clk_tck):
    present = [r for r in records if group in r]
    if not present:
        return None
    rss = sum(r[group]["Rss"] for r in present) / len(present) / 1024
    pss = sum(r[group]["Pss"] for r in present) / len(present) / 1024
    cores = None
    if len(present) > 1:
        seconds = present[-1]["t"] - present[0]["t"]
        ticks = present[-1][group]["cpu"] - present[0][group]["cpu"]
        if seconds > 0 and ticks >= 0:
            cores = ticks / clk_tck / seconds
    return rss, pss, cores


def main():
    import os
    path, groups = sys.argv[1], sys.argv[2:] or ["api", "media", "postgres", "redis"]
    clk_tck = os.sysconf("SC_CLK_TCK")
    print("phase".ljust(30), "n".rjust(4), *[f"{g[:24]:>26}" for g in groups])
    print("".ljust(35), *[f"{'RSS / PSS MB, cores':>26}" for _ in groups])
    for name, records in phases(path):
        cells = []
        for group in groups:
            s = summary(records, group, clk_tck)
            if s is None:
                cells.append(f"{'-':>26}")
            else:
                cores = "  -" if s[2] is None else f"{s[2]:4.2f}"
                cells.append(f"{s[0]:8.0f} / {s[1]:6.0f}, {cores:>5}")
        print(name[:30].ljust(30), str(len(records)).rjust(4), *cells)


if __name__ == "__main__":
    main()
