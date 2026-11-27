#!/usr/bin/env python3
"""Vets Dependabot pull requests before their commits are carried (skill
dependency-updates, step 2), and lists their updates.

  python3 -I .claude/skills/dependency-updates/vet.py <number>...

Run from the repository root. For each pull request it fetches
refs/pull/<n>/head and checks that it comes from Dependabot in this
repository, that every commit is Dependabot's own (sole author
dependabot[bot], one parent, verified and committed by web-flow), that it
touches only manifest, lock, image and pin files, and that each changed line
is a version line of its file. The updates come from the commits'
`updated-dependencies` blocks, never from the pull request's text.

Prints one JSON object per pull request: number, head, commits, security
(no update belongs to a Dependabot group: all version updates are grouped),
files and updates [{name, from, to, type, update_type, group}]. Exit status 1,
with every reason on stderr, when any pull request fails a check; then
nothing of the run may be carried or closed.
"""

import json
import re
import subprocess
import sys

BOT = "dependabot[bot]"
CRATES_IO = '"registry+https://github.com/rust-lang/crates.io-index"'
NPM_REGISTRY = "https://registry.npmjs.org/"
# Keys a dependency's inline table in Cargo.toml may have in a change.
CARGO_TABLE_KEYS = {"version", "features", "default-features", "optional"}

FILES = [
    # (pattern, kind)
    (r"(backend|desktop)/([\w.-]+/)*Cargo\.toml", "cargo-manifest"),
    (r"(backend|desktop)/Cargo\.lock", "cargo-lock"),
    (r"web/package\.json", "npm-manifest"),
    (r"web/package-lock\.json", "npm-lock"),
    (r"(backend/api|backend/media|web|deploy/compose/minio|deploy/compose/mc)/Dockerfile", "dockerfile"),
    (r"deploy/compose/[\w.-]+\.ya?ml", "compose"),
    (r"\.github/workflows/[\w.-]+\.ya?ml", "workflow"),
    (r"deploy/compose/tests/requirements\.txt", "pip"),
]


def run(*cmd):
    return subprocess.run(cmd, check=True, capture_output=True, text=True).stdout


def gh_json(*args):
    return json.loads(run("gh", *args))


def kind_of(path):
    for pattern, kind in FILES:
        if re.fullmatch(pattern, path):
            return kind
    return None


def updates_of(message):
    """The updated-dependencies block and the `Updates x from a to b` lines."""
    block = message.split("updated-dependencies:", 1)
    if len(block) != 2:
        return None
    entries, current = [], None
    for line in block[1].splitlines():
        if line.strip() in ("...", "---") or line.startswith("Signed-off-by"):
            break
        m = re.match(r"^- dependency-name: (\S+)\s*$", line)
        if m:
            current = {"name": m.group(1).strip("'\"")}
            entries.append(current)
            continue
        m = re.match(r"^  ([\w-]+): (.+?)\s*$", line)
        if m and current is not None:
            current[m.group(1)] = m.group(2).strip("'\"")
    froms = {}
    patterns = (
        r"^Updates `([^`]+)` from (\S+) to (\S+)$",
        r"^Bumps \[([^\]]+)\]\([^)]*\) from (\S+) to (\S+?)\.?$",
        r"^Bumps `?([^\s`\[]+)`? from `?([^\s`]+)`? to `?([^\s`]+?)`?\.?$",
    )
    for pattern in patterns:
        for m in re.finditer(pattern, message, re.M):
            froms.setdefault((m.group(1), m.group(3)), m.group(2))
    seen, out = set(), []
    for e in entries:
        key = (e["name"], e.get("dependency-version"))
        if key in seen:
            continue
        seen.add(key)
        out.append({
            "name": e["name"],
            "from": froms.get(key),
            "to": e.get("dependency-version"),
            "type": e.get("dependency-type"),
            "update_type": e.get("update-type"),
            "group": e.get("dependency-group"),
        })
    return out


def line_ok(kind, line, names):
    """Whether one added or removed line is a version line of its file."""
    text = line[1:]
    if kind in ("cargo-lock",):
        return not (line.startswith("+") and text.startswith("source = ") and text != "source = " + CRATES_IO)
    if kind == "npm-lock":
        m = re.match(r'\s*"resolved": "([^"]*)"', text)
        return not (line.startswith("+") and m and not m.group(1).startswith(NPM_REGISTRY))
    if kind == "dockerfile":
        return re.match(r"FROM\s", text) is not None
    if kind == "compose":
        return re.match(r"\s*image:\s", text) is not None
    if kind == "workflow":
        return re.match(r"\s*(-\s+)?uses:\s", text) is not None
    if kind == "pip":
        return re.fullmatch(r"([A-Za-z0-9_.-]+==\S+|\s*--hash=sha256:[0-9a-f]{64})\s*\\?\s*", text) is not None
    if kind == "npm-manifest":
        m = re.fullmatch(r'\s*"([^"]+)": "[^"]+",?\s*', text)
        return m is not None and m.group(1) in names
    if kind == "cargo-manifest":
        if re.fullmatch(r'\s*version\s*=\s*"[^"]*"\s*', text):
            return True
        m = re.fullmatch(r'\s*([\w-]+)\s*=\s*"[^"]*"\s*', text)
        if m:
            return m.group(1) in names
        m = re.fullmatch(r"\s*([\w-]+)\s*=\s*\{(.*)\}\s*", text)
        if m and m.group(1) in names:
            keys = set(re.findall(r"([\w-]+)\s*=", re.sub(r'"[^"]*"', '""', m.group(2))))
            return keys <= CARGO_TABLE_KEYS
        return False
    return False


def vet(number, problems):
    pr = gh_json("pr", "view", str(number), "--json",
                 "number,author,isCrossRepository,headRefName,headRefOid,state,commits")
    say = lambda msg: problems.append(f"#{number}: {msg}")
    if pr["state"] != "OPEN":
        say(f"state {pr['state']}")
    if pr["author"]["login"] not in ("app/dependabot", "dependabot", BOT):
        say(f"opened by {pr['author']['login']}")
    if pr["isCrossRepository"]:
        say("comes from a fork")
    if not pr["headRefName"].startswith("dependabot/"):
        say(f"branch {pr['headRefName']}")
    run("git", "fetch", "--quiet", "origin", f"+refs/pull/{number}/head:refs/dependabot-vet/{number}")
    head = run("git", "rev-parse", f"refs/dependabot-vet/{number}").strip()
    if head != pr["headRefOid"]:
        say(f"fetched head {head} is not the pull request's head {pr['headRefOid']}")
    repo = run("gh", "repo", "view", "--json", "nameWithOwner", "--jq", ".nameWithOwner").strip()
    commits, updates, files = [], [], set()
    for c in pr["commits"]:
        sha = c["oid"]
        commits.append(sha)
        authors = [a["login"] for a in c["authors"]]
        if authors != [BOT]:
            say(f"commit {sha[:10]} has authors {authors}")
        api = gh_json("api", f"repos/{repo}/commits/{sha}")
        if not api["commit"]["verification"]["verified"]:
            say(f"commit {sha[:10]} is not verified ({api['commit']['verification']['reason']})")
        if (api.get("committer") or {}).get("login") != "web-flow":
            say(f"commit {sha[:10]} committed by {(api.get('committer') or {}).get('login')}")
        if len(api["parents"]) != 1:
            say(f"commit {sha[:10]} has {len(api['parents'])} parents")
        message = run("git", "log", "-1", "--format=%B", sha)
        listed = updates_of(message)
        if not listed:
            say(f"commit {sha[:10]} has no updated-dependencies block")
            continue
        updates.extend(listed)
        names = {u["name"] for u in listed}
        path, kind, header = None, None, False
        for line in run("git", "show", "--format=", "-U0", "--no-color", "--no-ext-diff", sha).splitlines():
            if line.startswith("diff --git "):
                path, kind, header = None, None, True
            elif line.startswith("@@"):
                header = False
            elif header and (line.startswith("+++ ") or line.startswith("--- ")):
                name = line[4:]
                if name == "/dev/null":
                    say(f"commit {sha[:10]} adds or deletes a file")
                elif line.startswith("+++ "):
                    path = name[2:]
                    kind = kind_of(path)
                    files.add(path)
                    if kind is None:
                        say(f"commit {sha[:10]} changes {path}")
            elif header and line.startswith(("Binary files", "old mode", "new mode", "rename ", "similarity ", "deleted file", "new file")):
                say(f"commit {sha[:10]}: {line}")
            elif not header and kind and line[:1] in "+-":
                if not line_ok(kind, line, names):
                    say(f"commit {sha[:10]} {path}: not a version line: {line[:120]}")
    if not commits:
        say("has no commits")
    return {
        "number": number,
        "head": head,
        "commits": commits,
        "security": bool(updates) and all(u["group"] is None for u in updates),
        "files": sorted(files),
        "updates": updates,
    }


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    problems, results = [], []
    for arg in sys.argv[1:]:
        results.append(vet(int(arg), problems))
    for r in results:
        print(json.dumps(r))
    for p in problems:
        print("REJECTED " + p, file=sys.stderr)
    sys.exit(1 if problems else 0)


if __name__ == "__main__":
    main()
