#!/usr/bin/env python3
"""Vets Dependabot pull requests before their commits are carried (skill
dependency-updates, step 2), and lists their updates.

  python3 -I .claude/skills/dependency-updates/vet.py <number>...

Run from the repository root. A pull request passes when

- Dependabot opened it from a `dependabot/<ecosystem>/` branch of this
  repository, and the fetched head is the pull request's head;
- every commit has dependabot[bot] as its only author, one parent, a
  verified signature and web-flow as committer;
- its commit message lists the updates in an `updated-dependencies` block,
  each name and version made of `A-Za-z0-9@/._+~:-` only;
- it changes only these files, each only so:
  - Dockerfiles (`FROM`), compose files (`image:`), workflows (`uses:`,
    pinned to a 40-hex commit) and `deploy/compose/tests/requirements.txt`:
    each removed line has an added line that is the same except for the
    version (tag, digest, commit or version string), for a dependency the
    commit message names; no line is only added or only removed (hash lines
    of requirements.txt aside);
  - `Cargo.toml` and `web/package.json`, compared parsed: only the version
    of a dependency the commit message names changes (in a dependency
    table, as a string or a table's `version`);
  - `Cargo.lock`: every package new to the file comes from crates.io with a
    plain version; a package kept at its version keeps its source and
    checksum;
  - `web/package-lock.json`: outside `packages` nothing changes; every
    changed package has a plain version, an `integrity`, and a `resolved`
    URL that is the npm registry's tarball of that package and version; a
    package kept at its version keeps both; the root entry changes only the
    versions of dependencies the commit message names. A package bundled in
    its parent (`inBundle`, with neither `resolved` nor `integrity`, as npm
    writes it) passes when its parent is in the lock and not bundled itself.

Not checked, so CI's read-only build decides: which other packages a lock
file moves (Dependabot moves transitive dependencies with the named ones),
and whether a new version brings an install script (`hasInstallScript`:
this procedure never runs npm's scripts locally).

Prints one JSON object per pull request: number, head, ecosystem, commits,
files, updates [{name, from, to, type, update_type, group, security}] and
security. `from` comes from the commit message, else from the removed side
of the change. An update's `security` lists the GitHub advisories (public
advisory database) that affect its `from` version, or is null when `from`
is unknown or the database refuses the query; Docker and Compose images have
no advisory database, so theirs is always empty. The pull request's `security` is true when an update has an
advisory, "unknown" when one cannot be looked up, else false. Exit status 1, with every reason on stderr, when
any pull request fails; then nothing of the run is carried or closed. The
commits are fetched into a temporary ref that is deleted at the end; the
objects stay for the cherry-picks.
"""

import json
import os
import re
import subprocess
import sys
import tomllib

BOT = "dependabot[bot]"
CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"
NPM = "https://registry.npmjs.org/"
SAFE = re.compile(r"[A-Za-z0-9@/._+~:-]+")
SEMVER = re.compile(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?")
SHA = re.compile(r"[0-9a-f]{40}")
IMAGE = re.compile(r"([a-z0-9][a-z0-9./_-]*)(?::([\w][\w.-]*))?(?:@(sha256:[0-9a-f]{64}))?")
# Ecosystem in Dependabot's branch name -> GitHub advisory ecosystem.
ECOSYSTEMS = {
    "cargo": "rust", "npm_and_yarn": "npm", "pip": "pip", "github_actions": "actions",
    "docker": None, "docker_compose": None,
}
FILES = [
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


def pip_name(name):
    return re.sub(r"[-_.]+", "-", name).lower()


def updates_of(message):
    """The updated-dependencies block, with `from` from the message's lines."""
    block = message.split("\nupdated-dependencies:\n", 1)
    if len(block) != 2:
        return None
    entries, current = [], None
    for line in block[1].splitlines():
        if line.strip() in ("...", "---"):
            break
        m = re.fullmatch(r"- dependency-name: (.+?)\s*", line)
        if m:
            current = {"name": m.group(1).strip("'\"")}
            entries.append(current)
            continue
        m = re.fullmatch(r"  ([\w-]+): (.+?)\s*", line)
        if m and current is not None:
            current[m.group(1)] = m.group(2).strip("'\"")
    froms = {}
    for pattern in (
        r"^Updates `([^`]+)` from (\S+) to (\S+)$",
        r"^Bumps \[([^\]]+)\]\([^)]*\) from (\S+) to (\S+?)\.?$",
        r"^Bumps `?([^\s`\[]+)`? from `?([^\s`]+)`? to `?([^\s`]+?)`?\.?$",
    ):
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


def image_name(ref):
    m = IMAGE.fullmatch(ref)
    return m.group(1) if m else None


def line_key(kind, text, names):
    """A line with its version taken out, or None when it is no version line
    of a dependency the commit names."""
    if kind == "dockerfile":
        m = re.fullmatch(r"FROM\s+((?:--platform=\S+\s+)?)(\S+)((?:\s+AS\s+\S+)?)\s*", text, re.I)
        name = m and image_name(m.group(2))
        return f"FROM {m.group(1)}{name}{m.group(3)}" if name and name in names else None
    if kind == "compose":
        m = re.fullmatch(r"(\s*image:\s*)(\S+)\s*", text)
        name = m and image_name(m.group(2))
        return f"{m.group(1)}{name}" if name and name in names else None
    if kind == "workflow":
        m = re.fullmatch(r"(\s*(?:-\s+)?uses:\s+)([\w.-]+/[\w./-]+)@(\S+)(\s+#.*)?", text)
        if not m or not SHA.fullmatch(m.group(3)):
            return None
        action = m.group(2)
        repo = "/".join(action.split("/")[:2])
        return f"{m.group(1)}{action}" if action in names or repo in names else None
    if kind == "pip":
        m = re.fullmatch(r"([A-Za-z0-9_.-]+)==\S+(\s*\\?)\s*", text)
        named = {pip_name(n) for n in names}
        return f"{m.group(1)}==V{m.group(2)}" if m and pip_name(m.group(1)) in named else None
    return None


def line_name_version(kind, text):
    """(name, version) of a version line accepted by line_key."""
    if kind in ("dockerfile", "compose"):
        ref = text.split("image:", 1)[1] if kind == "compose" else re.sub(r"^FROM\s+(--platform=\S+\s+)?", "", text.strip()).split()[0]
        m = IMAGE.fullmatch(ref.strip())
        return (m.group(1), m.group(2) or m.group(3)) if m else (None, None)
    if kind == "workflow":
        m = re.fullmatch(r"\s*(?:-\s+)?uses:\s+([\w.-]+/[\w.-]+)[\w./-]*@(\S+)(?:\s+#\s*v?(\S+))?.*", text)
        return (m.group(1), m.group(3) or m.group(2)) if m else (None, None)
    if kind == "pip":
        m = re.fullmatch(r"([A-Za-z0-9_.-]+)==(\S+?)\s*\\?\s*", text)
        return (pip_name(m.group(1)), m.group(2)) if m else (None, None)
    return (None, None)


def show(sha, path):
    try:
        return run("git", "show", f"{sha}:{path}")
    except subprocess.CalledProcessError:
        return None


DEP_TABLES = ("dependencies", "dev-dependencies", "build-dependencies")


def check_cargo_manifest(old, new, names, say, froms):
    """Only the version of a named dependency changes."""
    def walk(a, b, path):
        if a == b:
            return
        if isinstance(a, dict) and isinstance(b, dict):
            for k in set(a) | set(b):
                if k not in a or k not in b:
                    say(f"Cargo.toml: {'.'.join(path + (k,))} added or removed")
                else:
                    walk(a[k], b[k], path + (k,))
            return
        named = (len(path) >= 2 and path[-2] in DEP_TABLES and path[-1] in names) or (
            len(path) >= 3 and path[-3] in DEP_TABLES and path[-2] in names and path[-1] == "version")
        if not (named and isinstance(a, str) and isinstance(b, str)):
            say(f"Cargo.toml: {'.'.join(path)} changed")
        else:
            froms.setdefault(path[-2] if path[-1] == "version" and path[-2] in names else path[-1], a)
    walk(tomllib.loads(old), tomllib.loads(new), ())


def check_npm_manifest(old, new, names, say, froms):
    """Only the version of a named dependency changes."""
    old, new = json.loads(old), json.loads(new)
    sections = ("dependencies", "devDependencies", "optionalDependencies", "peerDependencies")
    for field in set(old) | set(new):
        a, b = old.get(field), new.get(field)
        if a == b:
            continue
        if field not in sections or not isinstance(a, dict) or not isinstance(b, dict):
            say(f"package.json: {field} changed")
            continue
        for dep in set(a) | set(b):
            if a.get(dep) != b.get(dep) and (dep not in names or dep not in a or dep not in b):
                say(f"package.json: {field}.{dep} changed")
            elif a.get(dep) != b.get(dep):
                froms.setdefault(dep, a[dep])


def check_cargo_lock(old, new, say, froms):
    def packages(text):
        return {(p["name"], p["version"]): p for p in tomllib.loads(text).get("package", [])}
    before, after = packages(old), packages(new)
    for name, version in before:
        if (name, version) not in after:
            froms.setdefault(name, version)
    for key, p in after.items():
        if key in before:
            for field in ("source", "checksum"):
                if p.get(field) != before[key].get(field):
                    say(f"Cargo.lock: {key[0]} {key[1]} changed its {field}")
        else:
            if p.get("source") != CRATES_IO:
                say(f"Cargo.lock: new {key[0]} {key[1]} from {p.get('source')}")
            if not SEMVER.fullmatch(p["version"]):
                say(f"Cargo.lock: new {key[0]} has version {p['version']}")


def check_npm_lock(old, new, names, say, froms):
    old, new = json.loads(old), json.loads(new)
    if {k: v for k, v in old.items() if k != "packages"} != {k: v for k, v in new.items() if k != "packages"}:
        say("package-lock.json: changes outside packages")
    before, after = old.get("packages", {}), new.get("packages", {})
    for key, entry in after.items():
        if before.get(key) == entry:
            continue
        if key == "":
            root_old = before.get("", {})
            for field in set(entry) | set(root_old):
                if field in ("dependencies", "devDependencies", "optionalDependencies", "peerDependencies"):
                    a, b = root_old.get(field, {}), entry.get(field, {})
                    for dep in set(a) | set(b):
                        if a.get(dep) != b.get(dep) and (dep not in names or dep not in a or dep not in b):
                            say(f"package-lock.json: root {field} {dep} changed")
                elif root_old.get(field) != entry.get(field):
                    say(f"package-lock.json: root {field} changed")
            continue
        if entry.get("link"):
            say(f"package-lock.json: {key} became a link")
            continue
        if entry.get("inBundle") and "resolved" not in entry and "integrity" not in entry:
            parent = key.rsplit("/node_modules/", 1)[0]
            if parent == key or parent not in after or after[parent].get("inBundle"):
                say(f"package-lock.json: {key} is bundled without a parent that ships it")
            continue
        name = key.rsplit("node_modules/", 1)[-1]
        version = entry.get("version", "")
        if not SEMVER.fullmatch(version):
            say(f"package-lock.json: {key} has version {version!r}")
            continue
        tarball = f"{NPM}{name}/-/{name.split('/')[-1]}-{version}.tgz"
        if entry.get("resolved") != tarball:
            say(f"package-lock.json: {key} resolves to {entry.get('resolved')!r}")
        if not entry.get("integrity"):
            say(f"package-lock.json: {key} has no integrity")
        previous = before.get(key)
        if previous and previous.get("version") and previous.get("version") != version:
            froms.setdefault(name, previous["version"])
        if previous and previous.get("version") == version and previous.get("integrity") != entry.get("integrity"):
            say(f"package-lock.json: {key} {version} changed its integrity")


def check_commit(sha, names, say, froms):
    files = set()
    diff = run("git", "show", "--format=", "-U0", "--no-color", "--no-ext-diff", "--no-renames", sha)
    removed, added, path, kind, header = {}, {}, None, None, False
    for line in diff.splitlines():
        if line.startswith("diff --git "):
            path, kind, header = None, None, True
        elif line.startswith("@@"):
            header = False
        elif header:
            if line.startswith(("new file", "deleted file", "old mode", "new mode", "Binary files")):
                say(f"commit {sha[:10]}: {line}")
            elif line.startswith("+++ "):
                path = line[6:] if line.startswith("+++ b/") else None
                kind = kind_of(path) if path else None
                if path:
                    files.add(path)
                    if kind is None:
                        say(f"commit {sha[:10]} changes {path}")
        elif kind and line[:1] in "+-" and kind in ("dockerfile", "compose", "workflow", "pip"):
            text = line[1:]
            if kind == "pip" and re.fullmatch(r"\s*--hash=sha256:[0-9a-f]{64}\s*\\?\s*", text):
                continue
            key = line_key(kind, text, names)
            if key is None:
                say(f"commit {sha[:10]} {path}: not a version line: {line[:120]}")
                continue
            (added if line[0] == "+" else removed).setdefault(path, []).append(key)
            if line[0] == "-":
                name, version = line_name_version(kind, text)
                if name:
                    froms.setdefault(name, version)
    for p in set(removed) | set(added):
        if sorted(removed.get(p, [])) != sorted(added.get(p, [])):
            say(f"commit {sha[:10]} {p}: lines added or removed beyond a version change")
    # Lock files first: a `from` read off a lock is a version, a manifest's may be a range.
    for p in sorted(files, key=lambda p: (kind_of(p) not in ("cargo-lock", "npm-lock"), p)):
        kind = kind_of(p)
        if kind in ("cargo-lock", "npm-lock", "cargo-manifest", "npm-manifest"):
            old, new = show(f"{sha}^", p), show(sha, p)
            if old is None or new is None:
                continue
            report = lambda m, p=p: say(f"commit {sha[:10]} {p}: {m}")
            try:
                if kind == "cargo-lock":
                    check_cargo_lock(old, new, report, froms)
                elif kind == "npm-lock":
                    check_npm_lock(old, new, names, report, froms)
                elif kind == "cargo-manifest":
                    check_cargo_manifest(old, new, names, report, froms)
                else:
                    check_npm_manifest(old, new, names, report, froms)
            except (ValueError, KeyError, TypeError) as e:
                say(f"commit {sha[:10]} {p}: unreadable ({e})")
    return files


def advisories(ecosystem, name, version):
    """GHSA ids affecting name@version, or None when the database refuses the query."""
    try:
        found = gh_json("api", "-X", "GET", "/advisories", "-f", f"ecosystem={ecosystem}",
                        "-f", f"affects={name}@{version}")
    except (subprocess.CalledProcessError, ValueError):
        return None
    return [a.get("ghsa_id") for a in found]


def vet(number, problems, ref):
    say = lambda msg: problems.append(f"#{number}: {msg}")
    pr = gh_json("pr", "view", str(number), "--json",
                 "number,author,isCrossRepository,headRefName,headRefOid,state,commits")
    if pr["state"] != "OPEN":
        say(f"state {pr['state']}")
    if pr["author"]["login"] not in ("app/dependabot", "dependabot", BOT):
        say(f"opened by {pr['author']['login']}")
    if pr["isCrossRepository"]:
        say("comes from a fork")
    parts = pr["headRefName"].split("/")
    ecosystem = parts[1] if len(parts) > 2 and parts[0] == "dependabot" else None
    if ecosystem not in ECOSYSTEMS:
        say(f"branch {pr['headRefName']}")
    run("git", "fetch", "--quiet", "--no-tags", "origin", f"+refs/pull/{number}/head:{ref}")
    head = run("git", "rev-parse", ref).strip()
    if head != pr["headRefOid"]:
        say(f"fetched head {head} is not the pull request's head {pr['headRefOid']}")
    repo = gh_json("repo", "view", "--json", "nameWithOwner")["nameWithOwner"]
    commits, updates, files = [], [], set()
    for c in pr["commits"]:
        sha = c["oid"]
        commits.append(sha)
        authors = [a["login"] for a in c["authors"]]
        if authors != [BOT]:
            say(f"commit {sha[:10]} has authors {authors}")
        api = gh_json("api", f"repos/{repo}/commits/{sha}")
        verification = api["commit"]["verification"]
        if not verification["verified"]:
            say(f"commit {sha[:10]} is not verified ({verification['reason']})")
        if (api.get("committer") or {}).get("login") != "web-flow":
            say(f"commit {sha[:10]} committed by {(api.get('committer') or {}).get('login')}")
        if len(api["parents"]) != 1:
            say(f"commit {sha[:10]} has {len(api['parents'])} parents")
            continue
        listed = updates_of(run("git", "log", "-1", "--format=%B", sha))
        if not listed:
            say(f"commit {sha[:10]} has no updated-dependencies block")
            continue
        unsafe = [v for u in listed for v in (u["name"], u["from"], u["to"]) if v is not None and not SAFE.fullmatch(v)]
        if unsafe or any(u["to"] is None for u in listed):
            say(f"commit {sha[:10]} names unsafe or missing values: {unsafe!r}")
            continue
        froms = {}
        files |= check_commit(sha, {u["name"] for u in listed}, say, froms)
        for u in listed:
            if u["from"] is None:
                found = froms.get(u["name"], froms.get(pip_name(u["name"])))
                u["from"] = found if found and SAFE.fullmatch(found) else None
        updates.extend(listed)
    if not commits:
        say("has no commits")
    if not any(p.startswith(f"#{number}:") for p in problems):
        for u in updates:
            if ECOSYSTEMS.get(ecosystem) is None:
                u["security"] = []
            elif u["from"] is None:
                u["security"] = None
            else:
                u["security"] = advisories(ECOSYSTEMS[ecosystem], u["name"], u["from"])
    marks = [u.get("security", []) for u in updates]
    return {
        "number": number,
        "head": head,
        "ecosystem": ecosystem,
        "commits": commits,
        "security": True if any(marks) else "unknown" if None in marks else False,
        "files": sorted(files),
        "updates": updates,
    }


def main():
    if len(sys.argv) < 2 or not all(a.isdigit() for a in sys.argv[1:]):
        sys.exit(__doc__)
    problems, results = [], []
    ref = f"refs/dependabot-vet-{os.getpid()}"
    try:
        for arg in sys.argv[1:]:
            results.append(vet(int(arg), problems, ref))
    finally:
        subprocess.run(["git", "update-ref", "-d", ref], capture_output=True)
    for r in results:
        print(json.dumps(r))
    for p in problems:
        print("REJECTED " + p, file=sys.stderr)
    sys.exit(1 if problems else 0)


if __name__ == "__main__":
    main()
