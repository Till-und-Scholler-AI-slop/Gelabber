"""Mutation testing of the web client's changed lines (mutation-web.yml,
GEL-388). It informs and gates nothing.

`target` finds what to test and the release to compare it with, and fetches
both into the shallow checkout. The release is GitHub's Latest, as
image_set.py and desktop-release.yml read it. What is tested: a pull
request's merge commit (GITHUB_SHA), the branch a manual run names
(INPUT_REF), or else the development line, the newest version branch
(AGENTS.md). Writes base=, sha=, branch= and label= to $GITHUB_OUTPUT.

`plan BASE SHA` lists the lines of web/src that SHA adds or changes against
BASE as Stryker `--mutate` patterns (`src/x.ts:3-9`), in path order; a file
that would take the plan past MAX_FILES files or MAX_LINES lines is left out
and named in the summary, and smaller files after it still go in. Writes
plan.json and mutate= to $GITHUB_OUTPUT.

`summary REPORT PLAN WEB` reads Stryker's JSON report: the mutants no test
detected, by file and then by line, go to the job summary, and issue.json
holds what `issue` needs. WEB is the tested web/ folder, to tell which
files only browser tests cover.

`issue ISSUE_JSON STATE_OUT [--dry-run]` (scheduled runs, with `issues:
write` and `actions: read`) keeps one issue. Its state, the issue's number
and the mutants' keys (file, mutator, original, mutant: no line), lives in
the artifact of the newest earlier scheduled run, not in the issue body,
which the Linear sync may rewrite; STATE_OUT is this run's. Mutants not in
the last state are new. It opens the issue, reopens a closed one only for
new mutants, comments only on new ones and updates the list silently when
mutants only went away, and closes it when none is left. Only the newest
earlier scheduled run's state counts. Without one (none kept, expired or
malformed) it takes the newest issue with TITLE that this workflow opened
and counts every mutant as known, so a lost state cannot reopen or ping.
When every changed file was over the limits it does nothing and passes the
last state on as it was, lost included.
`--dry-run` reads and prints what it would do instead.
"""
from collections import Counter
import html
import json
import os
from pathlib import Path
import re
import subprocess
import sys

TITLE = 'Mutation testing: web mutants survive'
# Past these the plan cuts the diff, so that a release with a huge diff
# cannot run for hours; the job's timeout-minutes is the backstop. Measured
# on a runner (GEL-388): 709 lines gave 670 mutants, done in 3 min 6 s.
MAX_FILES = 60
MAX_LINES = 1500
# GitHub refuses an issue body above 65536 characters; the job summary takes
# 1 MiB. The full list is always in the run's artifact.
ISSUE_LIMIT = 60000
SUMMARY_LIMIT = 400000
# Mutants listed in the open, at most PER_FILE of a file; the rest go in a
# collapsed list.
SHOWN = 50
PER_FILE = 5
# The issue's state between scheduled runs: an artifact, not the issue body,
# which the Linear sync may rewrite. Kept for the longest retention GitHub allows.
STATE_ARTIFACT = 'mutation-web-state'
STATE_FILE = 'mutation-state.json'
STATE_DAYS = 90
SNIPPET = 60
SOURCE = re.compile(r'web/src/[A-Za-z0-9._/-]+\.tsx?')
# Translations are data, not logic: every string would be a mutant.
LEFT_OUT = ('web/src/i18n/catalogs/',)
BRANCH = re.compile(r'[A-Za-z0-9][A-Za-z0-9._/-]*')
RELEASE = re.compile(r'v\d+\.\d+\.\d+')
UNDETECTED = ('Survived', 'NoCoverage')
DETECTED = ('Killed', 'Timeout')
LINEAR = 'This issue reaches Linear without a project: add it to the project Gelabber, label Improvement.'
TODO = ('**What to do with a mutant below:** write a test that fails on it, or say in a comment here '
        'that it is equivalent or does not matter.')
READING = """\
**Reading this.** The run informs; it gates nothing. A surviving mutant is a \
change to a changed line that no node test noticed; an unreached one sits \
on a line no node test runs. Some are equivalent mutants: they behave like \
the original, and no test can kill them. A kill only shows that some test's \
result changed, so a test that pins today's output kills mutants as well as \
a test of the right behaviour. A timeout counts as detected: a mutant that \
loops for ever is ended by Stryker's timeout. Only the node tests run; the \
Chromium tests (`*.browser.test.tsx`) may catch some. In GEL-388's sample, \
59 of 94 surviving markdown mutants and 14 of 27 surviving ws/protocol \
mutants changed what a user or another program sees."""


def run(*args, stdin=None):
    try:
        return subprocess.run(args, check=True, capture_output=True, text=True, input=stdin).stdout
    except subprocess.CalledProcessError as error:
        print(error.stderr, file=sys.stderr, end='')
        raise


def output(**values):
    """Step outputs; a value never holds a newline."""
    path = os.environ.get('GITHUB_OUTPUT')
    lines = ''.join(f'{key}={value}\n' for key, value in values.items())
    if path:
        with open(path, 'a', encoding='utf-8') as out:
            out.write(lines)
    else:
        print(lines, end='')


def version_key(name):
    return tuple(int(part) for part in name[1:].split('.'))


def newest_version_branch(ls_remote):
    """The development line from `git ls-remote --heads` output: the version
    branch with the highest version, as AGENTS.md sorts `origin/v[0-9]*`."""
    names = [m[1] for line in ls_remote.splitlines()
             if (m := re.fullmatch(r'[0-9a-f]{40}\trefs/heads/(v\d+(?:\.\d+)*)', line.strip()))]
    if not names:
        raise ValueError('no version branch on origin')
    return max(names, key=version_key)


def choose(event, input_ref, ls_remote):
    """The branch to test, or None for the pull request's merge commit.
    `ls_remote` returns `git ls-remote --heads origin` output."""
    if event == 'pull_request':
        return None
    if input_ref:
        if not BRANCH.fullmatch(input_ref) or '..' in input_ref:
            raise ValueError(f'not a branch name: {input_ref!r}')
        return input_ref
    return newest_version_branch(ls_remote())


def target():
    event, input_ref = os.environ['EVENT'], os.environ.get('INPUT_REF', '').strip()
    repo = os.environ['GITHUB_REPOSITORY']
    base = run('gh', 'release', 'view', '--repo', repo, '--json', 'tagName', '--jq', '.tagName').strip()
    if not RELEASE.fullmatch(base):
        raise ValueError(f'the latest release is {base!r}, not vX.Y.Z')
    run('git', 'fetch', '--no-tags', '--depth=1', 'origin', f'+refs/tags/{base}:refs/tags/{base}')
    branch = choose(event, input_ref, lambda: run('git', 'ls-remote', '--heads', 'origin', 'refs/heads/v*'))
    if branch is None:
        sha, label = os.environ['GITHUB_SHA'], 'the pull request'
    else:
        run('git', 'fetch', '--no-tags', '--depth=1', 'origin',
            f'+refs/heads/{branch}:refs/remotes/origin/{branch}')
        sha, label = run('git', 'rev-parse', f'refs/remotes/origin/{branch}').strip(), f'`{branch}`'
    output(base=base, sha=sha, branch=branch or '', label=label)


def changed_lines(diff):
    """{path: [(first, last), ...]} of the lines a `git diff -U0` adds or
    changes, by file of the new side."""
    found, path = {}, None
    for line in diff.splitlines():
        if line.startswith('+++ '):
            path = line[6:] if line.startswith('+++ b/') else None
        elif line.startswith('@@ ') and path:
            m = re.match(r'@@ -\S+ \+(\d+)(?:,(\d+))? @@', line)
            first, count = int(m[1]), int(m[2] if m[2] is not None else 1)
            if count:
                found.setdefault(path, []).append((first, first + count - 1))
    return found


def mutable(path):
    """Why a changed file under web/src is not mutated, or None."""
    if not SOURCE.fullmatch(path):
        return 'not TypeScript source' if re.search(r'\.(?:ts|tsx)$', path) is None else 'unusual characters'
    if '.test.' in path or path.endswith('.d.ts'):
        return 'a test or declaration'
    if path.startswith(LEFT_OUT):
        return 'translations'
    return None


def plan_from_diff(diff, max_files=MAX_FILES, max_lines=MAX_LINES):
    """The files and lines to mutate and those left out, in path order."""
    files, skipped, total = [], [], 0
    # git diff -U0 never gives two hunks that touch, so the ranges stay apart.
    for path, ranges in sorted(changed_lines(diff).items()):
        lines = sum(last - first + 1 for first, last in ranges)
        reason = mutable(path)
        if reason is None and len(files) >= max_files:
            reason = f'over the limit of {max_files} files'
        elif reason is None and total + lines > max_lines:
            reason = f'over the limit of {max_lines} changed lines'
        if reason:
            if reason.startswith('over'):
                skipped.append({'path': path, 'lines': lines, 'reason': reason})
            continue
        files.append({'path': path, 'lines': lines, 'ranges': ranges})
        total += lines
    return {'files': files, 'skipped': skipped, 'lines': total,
            'limits': {'files': max_files, 'lines': max_lines}}


def patterns(plan):
    """Stryker's --mutate value, relative to web/."""
    return ','.join(f'{f["path"].removeprefix("web/")}:{first}-{last}'
                    for f in plan['files'] for first, last in f['ranges'])


def plan(base, sha):
    if not RELEASE.fullmatch(base) or not re.fullmatch(r'[0-9a-f]{40}', sha):
        raise ValueError('plan needs a release tag and a commit SHA')
    diff = run('git', 'diff', '--no-color', '--no-ext-diff', '--no-renames', '-U0',
               f'refs/tags/{base}', sha, '--', 'web/src')
    result = plan_from_diff(diff, MAX_FILES, MAX_LINES)
    result.update(base=base, sha=sha, label=os.environ.get('LABEL', sha[:12]))
    Path('plan.json').write_text(json.dumps(result, indent=1) + '\n', encoding='utf-8')
    for item in result['skipped']:
        print(f'::warning title=File left out of the mutation run::{item["path"]} ({item["lines"]} lines): '
              f'{item["reason"]}')
    output(mutate=patterns(result), files=len(result['files']))
    print(f'{len(result["files"])} files, {result["lines"]} changed lines to mutate; '
          f'{len(result["skipped"])} files left out by the limits.')


def plural(count, word):
    return f'{count} {word}' + ('' if count == 1 else 's')


def snippets(before, after):
    """Original and mutant as table cells, each at most SNIPPET characters,
    cut around the part where they differ so the two never read the same."""
    before, after = ' '.join(before.split()), ' '.join(after.split())
    start = 0
    if max(len(before), len(after)) > SNIPPET:
        same = 0
        while same < min(len(before), len(after)) and before[same] == after[same]:
            same += 1
        start = max(0, same - SNIPPET // 3)

    def cell(text):
        part = text[start:start + SNIPPET - 2]
        part = ('…' if start else '') + part + ('…' if start + SNIPPET - 2 < len(text) else '')
        return '<code>' + html.escape(part).replace('|', '&#124;') + '</code>'
    return cell(before), cell(after)


def original(source, location):
    lines = source.splitlines()
    start, end = location['start'], location['end']
    if start['line'] == end['line']:
        return lines[start['line'] - 1][start['column'] - 1:end['column'] - 1]
    part = [lines[start['line'] - 1][start['column'] - 1:]] + lines[start['line']:end['line'] - 1]
    return '\n'.join(part + [lines[end['line'] - 1][:end['column'] - 1]])


def undetected(report):
    """(counts by status, [(file, line, column, status, mutator, original,
    replacement)] of the mutants no test detected), in file and line order."""
    counts, found = {}, []
    for name, data in sorted(report['files'].items()):
        for mutant in data['mutants']:
            counts[mutant['status']] = counts.get(mutant['status'], 0) + 1
            if mutant['status'] in UNDETECTED:
                start = mutant['location']['start']
                found.append((name, start['line'], start['column'], mutant['status'], mutant['mutatorName'],
                              original(data['source'], mutant['location']), mutant.get('replacement', '')))
    return counts, sorted(found)


def mutant_key(item):
    """What a mutant is, without its line: an edit above it does not make it
    new."""
    name, _, _, _, mutator, before, after = item
    return '\x1f'.join((name, mutator, ' '.join(before.split()), ' '.join(after.split())))


def new_count(current, known):
    """How many of the current keys (a multiset) are not among the known."""
    return sum((Counter(current) - Counter(known)).values())


def browser_only(web, name):
    """The browser test of a source file (`src/x.tsx`) when it has one and
    no node test of the same name, else None."""
    stem = Path(web) / re.sub(r'\.tsx?$', '', name)
    browser = stem.with_name(stem.name + '.browser.test.tsx')
    node = [stem.with_name(stem.name + f'.test.{ext}') for ext in ('ts', 'tsx', 'mjs', 'js')]
    return browser.name if browser.is_file() and not any(path.is_file() for path in node) else None


def by_file(found):
    """{file: [mutants]}, the file with the most undetected mutants first."""
    files = {}
    for item in found:
        files.setdefault(item[0], []).append(item)
    return dict(sorted(files.items(), key=lambda entry: (-len(entry[1]), entry[0])))


def file_table(found, web):
    rows = ['| File | Survived | No node test reaches it | Note |', '|---|---|---|---|']
    for name, items in by_file(found).items():
        unreached = sum(item[3] == 'NoCoverage' for item in items)
        only = browser_only(web, name)
        note = f'tested only in Chromium (`{only}`), which this run leaves out' if only else ''
        rows.append(f'| `web/{name}` | {len(items) - unreached} | {unreached} | {note} |')
    return rows


def mutant_row(item):
    name, line, _, status, mutator, before, after = item
    shown_before, shown_after = snippets(before, after)
    return (f'| `web/{name}` | {line} | {"unreached" if status == "NoCoverage" else "survived"} '
            f'| {mutator} | {shown_before} | {shown_after} |')


def mutant_rows(found, budget):
    """The mutant list: in the file table's order, at most PER_FILE of each
    file and SHOWN in all in the open, the rest collapsed, within `budget`
    characters; the last line says how many did not fit."""
    head = ['| File | Line | Status | Mutator | Original | Mutant |', '|---|---|---|---|---|---|']
    groups = list(by_file(found).values())
    first = [item for items in groups for item in items[:PER_FILE]][:SHOWN]
    chosen = set(map(id, first))
    rest = [item for items in groups for item in items if id(item) not in chosen]
    lines = head + [mutant_row(item) for item in first]
    size = sum(len(line) + 1 for line in lines)
    if size > budget:
        raise ValueError('the budget does not hold the first rows')
    if rest:
        opening = ['', f'<details><summary>{plural(len(rest), "more mutant")}</summary>', ''] + head
        size += sum(len(line) + 1 for line in opening) + 200
        kept = []
        for item in rest:
            row = mutant_row(item)
            if size + len(row) + 1 > budget:
                break
            kept.append(row)
            size += len(row) + 1
        if kept:
            lines += opening + kept + ['', '</details>']
        if len(kept) < len(rest):
            lines += ['', f'{len(rest) - len(kept)} more are in the run\'s artifact `mutation-web`.']
    return lines


def nothing_planned(plan_data):
    if plan_data['skipped']:
        return 'Every changed file was over the limits; nothing was mutated.'
    return f'No mutable `web/src` lines changed since {plan_data["base"]}.'


def report_text(plan_data, counts, found, limit, web):
    total = sum(counts.values())
    detected = sum(counts.get(s, 0) for s in DETECTED)
    valid = detected + sum(counts.get(s, 0) for s in UNDETECTED)
    score = f'{100 * detected / valid:.1f} %' if valid else 'none'
    parts = [
        f'{plan_data["label"]} at `{plan_data["sha"][:12]}`, the lines of `web/src` changed since '
        f'{plan_data["base"]}: {plural(len(plan_data["files"]), "file")}, {plural(plan_data["lines"], "line")}, '
        f'{plural(total, "mutant")}.',
        '',
        '| Killed | Timeout | Survived | No node test reaches it | Other | Detected |',
        '|---|---|---|---|---|---|',
        f'| {counts.get("Killed", 0)} | {counts.get("Timeout", 0)} | {counts.get("Survived", 0)} '
        f'| {counts.get("NoCoverage", 0)} | {total - valid} | {score} |',
        '',
    ]
    if plan_data['skipped']:
        parts += [f'**Left out:** {plural(len(plan_data["skipped"]), "file")} would have taken the run past '
                  f'{plan_data["limits"]["files"]} files or {plan_data["limits"]["lines"]} lines: '
                  + ', '.join(f'`{s["path"]}` ({s["lines"]} lines)' for s in plan_data['skipped']) + '.', '']
    if not plan_data['files']:
        parts += [nothing_planned(plan_data), '']
    elif not found:
        parts += ['A test detected every mutant of the changed lines.', '']
    else:
        parts += [f'### {plural(len(found), "mutant")} no test detected, by file', '']
        parts += file_table(found, web) + ['', TODO, '', '### By line', '']
        budget = limit - sum(len(p) + 1 for p in parts) - len(READING) - 10
        parts += mutant_rows(found, budget) + ['']
    return '\n'.join(parts + [READING])


def summary(report_path, plan_path, web):
    plan_data = json.loads(Path(plan_path).read_text(encoding='utf-8'))
    if Path(report_path).exists():
        counts, found = undetected(json.loads(Path(report_path).read_text(encoding='utf-8')))
    elif not plan_data['files']:
        counts, found = {}, []
    else:
        raise FileNotFoundError(f'no Stryker report at {report_path}')
    text = '## Web mutation testing\n\n' + report_text(plan_data, counts, found, SUMMARY_LIMIT, web)
    path = os.environ.get('GITHUB_STEP_SUMMARY')
    if path:
        with open(path, 'a', encoding='utf-8') as out:
            out.write(text + '\n')
    else:
        print(text)
    Path('issue.json').write_text(json.dumps({
        'count': len(found), 'survived': counts.get('Survived', 0), 'unreached': counts.get('NoCoverage', 0),
        'files': len(plan_data['files']), 'skipped': len(plan_data['skipped']), 'base': plan_data['base'],
        'sha': plan_data['sha'], 'keys': [mutant_key(item) for item in found],
        'body': report_text(plan_data, counts, found, ISSUE_LIMIT - 1000, web),
    }) + '\n', encoding='utf-8')
    output(undetected=len(found))


def decide(existing, info, known, run_url):
    """What `issue` does: a list of ('create', None, body), ('reopen',
    number, None), ('edit', number, body), ('comment', number, text) and
    ('close', number, None). `existing` is the issue's number and state (OPEN
    or CLOSED), or None; `known` the mutant keys of the last scheduled run,
    or None when its state is lost: then every mutant counts as known."""
    sha = info['sha'][:12]
    is_open = existing is not None and existing['state'] == 'OPEN'
    if info['count'] == 0:
        if info['files'] == 0 and info['skipped']:
            return []
        if not is_open:
            return []
        text = (f'No mutable `web/src` lines changed since {info["base"]} at {sha}, so nothing was mutated: '
                f'{run_url}' if info['files'] == 0
                else f'A test detects every mutant of the changed lines at {sha}: {run_url}')
        return [('comment', existing['number'], text), ('close', existing['number'], None)]
    body = f'[Scheduled run]({run_url}); the full report is its artifact `mutation-web`.\n\n{info["body"]}\n\n{LINEAR}\n'
    if existing is None:
        return [('create', None, body)]
    number = existing['number']
    new = 0 if known is None else new_count(info['keys'], known)
    if new:
        text = (f'{plural(new, "new mutant")} no test detects at {sha} ({info["count"]} in all: '
                f'{info["survived"]} survived, {info["unreached"]} unreached): {run_url}. The description lists them.')
        return ([] if is_open else [('reopen', number, None)]) + [('edit', number, body), ('comment', number, text)]
    if is_open and (known is None or Counter(info['keys']) != Counter(known)):
        # Only fewer mutants, or the state was lost: the list, without a ping.
        return [('edit', number, body)]
    return []


def find_issue(repo):
    """The newest issue titled TITLE that this workflow opened, open or
    closed: in a public repository anyone can open one with that title."""
    found = json.loads(run('gh', 'issue', 'list', '--repo', repo, '--state', 'all', '--limit', '20',
                           '--search', f'in:title "{TITLE}" author:app/github-actions sort:created-desc',
                           '--json', 'number,title,state'))
    match = [item for item in found if item['title'] == TITLE]
    return max(match, key=lambda item: item['number']) if match else None


def issue_state(repo, number):
    """The issue the last state names, or None when it cannot be read or
    is not this workflow's (a wrong or transferred number)."""
    try:
        data = json.loads(run('gh', 'issue', 'view', str(number), '--repo', repo, '--json', 'number,state,title'))
    except subprocess.CalledProcessError:
        return None
    return data if data.get('number') == number and data.get('title') == TITLE else None


def previous_state(repo, run_id, work):
    """{issue, keys} that the newest earlier successful scheduled run kept in
    its artifact STATE_ARTIFACT, or None: none kept, expired after
    STATE_DAYS, unreadable, or a lost state passed on (`keys` null). Only that
    run counts: an older state would make mutants new that are not."""
    try:
        runs = json.loads(run('gh', 'run', 'list', '--repo', repo, '--workflow', 'mutation-web.yml',
                              '--event', 'schedule', '--status', 'success', '--limit', '5',
                              '--json', 'databaseId'))
    except subprocess.CalledProcessError:
        return None
    earlier = [item['databaseId'] for item in runs if str(item['databaseId']) != str(run_id)]
    if not earlier:
        return None
    folder = Path(work) / str(earlier[0])
    try:
        run('gh', 'run', 'download', str(earlier[0]), '--repo', repo, '--name', STATE_ARTIFACT,
            '--dir', str(folder))
        state = json.loads((folder / STATE_FILE).read_text(encoding='utf-8'))
    except (subprocess.CalledProcessError, OSError, ValueError):
        return None
    if (isinstance(state, dict) and isinstance(state.get('keys'), list)
            and all(isinstance(key, str) for key in state['keys'])
            and (state.get('issue') is None or type(state['issue']) is int)):
        return state
    return None


def issue(info_path, dry_run, state_out):
    repo = os.environ['GITHUB_REPOSITORY']
    run_id = os.environ['GITHUB_RUN_ID']
    run_url = f"{os.environ['GITHUB_SERVER_URL']}/{repo}/actions/runs/{run_id}"
    info = json.loads(Path(info_path).read_text(encoding='utf-8'))
    state = previous_state(repo, run_id, Path(state_out).parent / 'previous')
    existing = issue_state(repo, state['issue']) if state and state.get('issue') else None
    if existing is None:
        existing = find_issue(repo)
    known = state['keys'] if state else None
    actions = decide(existing, info, known, run_url)
    where = f'#{existing["number"]} ({existing["state"].lower()})' if existing else 'none'
    kept = 'none, so every mutant counts as known' if state is None else plural(len(known), 'mutant key')
    print(f'Issue "{TITLE}": {where}; last state: {kept}; {info["count"]} mutants undetected now.')
    if not actions:
        print('Nothing to do.')
    number = existing['number'] if existing else None
    for action, target_number, text in actions:
        if dry_run:
            label = f'#{target_number}' if target_number else 'an issue'
            print(f'Would {action} {label}' + (f':\n{text}\n' if text else ''))
        elif action == 'create':
            url = run('gh', 'issue', 'create', '--repo', repo, '--title', TITLE, '--body-file', '-', stdin=text)
            number = int(re.search(r'/issues/(\d+)', url)[1])
            print(f'#{number}: created')
        elif action in ('edit', 'comment'):
            run('gh', 'issue', action, str(target_number), '--repo', repo, '--body-file', '-', stdin=text)
            print(f'#{target_number}: {action}')
        else:
            run('gh', 'issue', action, str(target_number), '--repo', repo)
            print(f'#{target_number}: {action}')
    # Every changed file over the limits measured nothing: keep the last list,
    # or pass a lost state on as lost (null), so the next run pings nobody.
    keys = known if info['files'] == 0 and info['skipped'] else info['keys']
    Path(state_out).parent.mkdir(parents=True, exist_ok=True)
    Path(state_out).write_text(json.dumps({'issue': number, 'keys': keys}) + '\n', encoding='utf-8')


def main(argv):
    command, args = (argv[0], argv[1:]) if argv else ('', [])
    if command == 'target' and not args:
        target()
    elif command == 'plan' and len(args) == 2:
        plan(*args)
    elif command == 'summary' and len(args) == 3:
        summary(*args)
    elif command == 'issue' and len(args) in (2, 3) and args[1:2] != ['--dry-run'] and set(args[2:]) <= {'--dry-run'}:
        issue(args[0], '--dry-run' in args[2:], args[1])
    else:
        print('usage: mutation_web.py target | plan BASE SHA | summary REPORT PLAN WEB | issue ISSUE_JSON STATE_OUT [--dry-run]',
              file=sys.stderr)
        return 2
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
