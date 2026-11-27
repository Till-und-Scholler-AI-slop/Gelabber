#!/usr/bin/env python3
"""Checks that a hashed requirements file lists, for each pinned release,
exactly the SHA-256 of every file PyPI has for it (skill dependency-updates,
renew.sh pip). Reads PyPI's JSON API; installs and runs nothing.

  python3 -I .claude/skills/dependency-updates/pip_hashes.py <requirements.txt>
"""

import json
import re
import sys
import urllib.request

PIN = re.compile(r"^([A-Za-z0-9_.-]+)==(\S+)((?:[ \t]*\\\n[ \t]*--hash=sha256:[0-9a-f]{64})+)", re.M)


def pypi_files(name, version):
    with urllib.request.urlopen(f"https://pypi.org/pypi/{name}/{version}/json", timeout=60) as r:
        return {u["digests"]["sha256"] for u in json.load(r)["urls"]}


def problems(text, files_of=pypi_files):
    """One message per pin whose hashes differ from PyPI's files."""
    pins = PIN.findall(text)
    if not pins:
        return ["no hashed pin"]
    out = []
    for name, version, block in pins:
        have = set(re.findall(r"sha256:([0-9a-f]{64})", block))
        want = files_of(name, version)
        if have != want:
            out.append(f"{name} {version}: {len(have - want)} hashes PyPI does not have, "
                       f"{len(want - have)} PyPI files without a hash")
    return out


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    with open(sys.argv[1]) as f:
        found = problems(f.read())
    for p in found:
        print(f"pip_hashes: {sys.argv[1]}: {p}", file=sys.stderr)
    sys.exit(1 if found else 0)


if __name__ == "__main__":
    main()
