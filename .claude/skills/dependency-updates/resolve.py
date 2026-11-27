#!/usr/bin/env python3
"""Resolves a stopped cherry-pick of a Dependabot pull request whose
conflicts are only version lines (skill dependency-updates, step 4).

  python3 -I .claude/skills/dependency-updates/resolve.py <vet.jsonl> <number>

Run from the repository root while `git cherry-pick` stops on conflicts.
Dependabot builds on `main`; once an earlier week's update is on the
development line, the next week's moves the same lines again. In each
conflicted Dockerfile, compose file, workflow, Cargo.toml, package.json or
requirements.txt, every conflict hunk must pair the development line's lines
(ours) with the pull request's (theirs) one to one, each pair the same
dependency (one the pull request names, from vet.jsonl) with only its
version different; requirements.txt pairs a pin with its hashes. Each pair
takes the higher version (theirs on a tie, e.g. a newer digest of the same
tag). Resolved files are staged. Lock files are listed as `lock <path>` for
step 4's lock rule. Exit status 1, with the reason, when any hunk is not
such a pair; then the pick is aborted and the pull request waits a week.
Writes nothing but the conflicted files; runs nothing else.
"""

import importlib.util
import json
from pathlib import Path
import re
import subprocess
import sys

spec = importlib.util.spec_from_file_location("vet", Path(__file__).with_name("vet.py"))
vet = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vet)

HASH = re.compile(r"\s*--hash=sha256:[0-9a-f]{64}\s*\\?\s*")


def version_key(version):
    return [(0, int(t)) if t.isdigit() else (1, t) for t in re.findall(r"\d+|[A-Za-z]+", version)]


def manifest_line(kind, text, names):
    """(key, version) of a dependency line of Cargo.toml or package.json."""
    if kind == "cargo-manifest":
        m = re.fullmatch(r'(\s*)([\w-]+)(\s*=\s*)"([^"]*)"\s*', text)
        if m and m.group(2) in names:
            return f"{m.group(1)}{m.group(2)}{m.group(3)}", m.group(4)
        m = re.fullmatch(r"\s*([\w-]+)\s*=\s*\{.*\bversion\s*=\s*\"([^\"]*)\".*\}\s*", text)
        if m and m.group(1) in names:
            return re.sub(r'(\bversion\s*=\s*)"[^"]*"', r"\1V", text), m.group(2)
        return None, None
    m = re.fullmatch(r'(\s*)"([^"]+)": "([^"]+)"(,?)\s*', text)
    if m and m.group(2) in names:
        return f'{m.group(1)}"{m.group(2)}": V{m.group(4)}', m.group(3)
    return None, None


def entry(kind, lines, names):
    """(key, version) of one pairable unit: a line, or a pin with its hashes."""
    first = lines[0]
    if kind in ("cargo-manifest", "npm-manifest"):
        return manifest_line(kind, first, names)
    key = vet.line_key(kind, first, names)
    _, version = vet.line_name_version(kind, first)
    return (key, version) if key and version else (None, None)


def units(kind, lines):
    """Lines grouped into units: requirements.txt pins keep their hashes."""
    out = []
    for line in lines:
        if kind == "pip" and HASH.fullmatch(line) and out:
            out[-1].append(line)
        else:
            out.append([line])
    return out


def resolve_text(kind, text, names):
    """The text with every conflict hunk resolved, or raises ValueError."""
    out, lines, i = [], text.split("\n"), 0
    while i < len(lines):
        if not lines[i].startswith("<<<<<<< "):
            out.append(lines[i])
            i += 1
            continue
        ours, theirs, part = [], [], "ours"
        i += 1
        while not lines[i].startswith(">>>>>>> "):
            if lines[i].startswith("||||||| "):
                part = "base"
            elif lines[i] == "=======":
                part = "theirs"
            elif part == "ours":
                ours.append(lines[i])
            elif part == "theirs":
                theirs.append(lines[i])
            i += 1
        i += 1
        ours_units, theirs_units = units(kind, ours), units(kind, theirs)
        if len(ours_units) != len(theirs_units):
            raise ValueError(f"a hunk with {len(ours)} lines against {len(theirs)}")
        for mine, other in zip(ours_units, theirs_units):
            if mine == other:
                out.extend(mine)
                continue
            (key_o, v_o), (key_t, v_t) = entry(kind, mine, names), entry(kind, other, names)
            if key_o is None or key_o != key_t:
                raise ValueError(f"not the same dependency's version: {mine[0].strip()!r} / {other[0].strip()!r}")
            out.extend(mine if version_key(v_o) > version_key(v_t) else other)
    return "\n".join(out)


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    names = None
    for line in Path(sys.argv[1]).read_text().splitlines():
        record = json.loads(line)
        if str(record["number"]) == sys.argv[2]:
            names = {u["name"] for u in record["updates"]}
    if not names or not all(vet.SAFE.fullmatch(n) for n in names):
        sys.exit(f"resolve: no vetted updates for #{sys.argv[2]}")
    names |= {vet.pip_name(n) for n in names}
    conflicted = subprocess.run(["git", "diff", "--name-only", "--diff-filter=U"], check=True,
                                capture_output=True, text=True).stdout.split()
    failed = False
    for path in conflicted:
        kind = vet.kind_of(path)
        if kind in ("cargo-lock", "npm-lock"):
            print(f"lock {path}")
            continue
        try:
            if kind is None:
                raise ValueError("not a file Dependabot updates")
            resolved = resolve_text(kind, Path(path).read_text(), names)
        except ValueError as e:
            print(f"resolve: {path}: {e}", file=sys.stderr)
            failed = True
            continue
        Path(path).write_text(resolved)
        subprocess.run(["git", "add", "--", path], check=True)
        print(f"resolved {path}")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
