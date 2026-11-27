"""Mutation testing of the web client's changed lines (mutation-web.yml,
GEL-388). It informs and gates nothing.

`target` finds what to test and the release to compare it with, and fetches
both into the shallow checkout. The release is GitHub's Latest, as
image_set.py and desktop-release.yml read it. What is tested: a pull
request's merge commit (GITHUB_SHA), the branch a manual run names
(INPUT_REF), or else the development line, the newest version branch
(AGENTS.md). Writes base=, sha=, branch= and label= to $GITHUB_OUTPUT.

`plan BASE SHA` lists the lines of web/src that SHA adds or changes against
BASE as Stryker `--mutate` patterns (`src/x.ts:3-9`), at most MAX_FILES
files and MAX_LINES lines; the files past either limit are named in the
summary. Writes plan.json and mutate= to $GITHUB_OUTPUT.

`summary REPORT PLAN` reads Stryker's JSON report: the surviving mutants per
file and line go to the job summary, and issue.json holds what `issue`
needs.

`issue ISSUE_JSON [--dry-run]` (scheduled runs, with an `issues: write`
token) opens the one standing issue when mutants survive, replaces its list
when they changed, and closes it when none survive. `--dry-run` reads the
open issues and prints what it would do instead.
"""
import hashlib
import html
import json
import os
from pathlib import Path
import re
import subprocess
import sys

TITLE = 'Mutation testing: web mutants survive'
# Past these the plan cuts the diff, so that a release with a huge diff
# cannot run for hours; the job's timeout-minutes is the backstop.
MAX_FILES = 60
MAX_LINES = 1500
# GitHub refuses an issue body above 65536 characters; the job summary takes
# 1 MiB. The full list is always in the run's artifact.
ISSUE_LIMIT = 60000
SUMMARY_LIMIT = 400000
SNIPPET = 60
SOURCE = re.compile(r'web/src/[A-Za-z0-9._/-]+\.tsx?')
# Translations are data, not logic: every string would be a mutant.
LEFT_OUT = ('web/src/i18n/catalogs/',)
BRANCH = re.compile(r'[A-Za-z0-9][A-Za-z0-9._/-]*')
RELEASE = re.compile(r'v\d+\.\d+\.\d+')
FINGERPRINT = re.compile(r'<!-- mutation-web: ([0-9a-f]{64}) -->')
UNDETECTED = ('Survived', 'NoCoverage')
DETECTED = ('Killed', 'Timeout')
LINEAR = 'This issue reaches Linear without a project: add it to the project Gelabber, label Improvement.'
READING = """\
**Reading this.** The run informs; it gates nothing. A surviving mutant is a \
change to a changed line that no node test noticed. Some are equivalent \
mutants: they behave like the original, and no test can kill them. A kill \
only shows that some test's result changed, so a test that pins today's \
output kills mutants as well as a test of the right behaviour. A timeout \
counts as detected: a mutant that loops for ever is ended by Stryker's \
timeout. Only the node tests run; the Chromium tests may catch some \
survivors. In GEL-388's sample about 9 of every 100 web mutants survived \
although they changed what a user sees."""


def run(*args, stdin=None):
    return subprocess.run(args, check=True, capture_output=True, text=True, input=stdin).stdout


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
    result = plan_from_diff(diff)
    result.update(base=base, sha=sha, label=os.environ.get('LABEL', sha[:12]))
    Path('plan.json').write_text(json.dumps(result, indent=1) + '\n', encoding='utf-8')
    for item in result['skipped']:
        print(f'::warning title=Mutation run cut::{item["path"]} ({item["lines"]} lines) not mutated: '
              f'{item["reason"]}')
    output(mutate=patterns(result), files=len(result['files']))
    print(f'{len(result["files"])} files, {result["lines"]} changed lines to mutate; '
          f'{len(result["skipped"])} files over the limits.')


def plural(count, word):
    return f'{count} {word}' + ('' if count == 1 else 's')


def snippet(text):
    text = ' '.join(text.split())
    text = text if len(text) <= SNIPPET else text[:SNIPPET - 1] + '…'
    return '<code>' + html.escape(text).replace('|', '&#124;') + '</code>'


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


def fingerprint(found):
    text = '\n'.join(f'{f}:{line}:{status}:{mutator}:{replacement}'
                     for f, line, _, status, mutator, _, replacement in found)
    return hashlib.sha256(text.encode()).hexdigest()


def table(found, limit):
    """Rows of the survivors table within `limit` characters, and how many
    did not fit."""
    rows = ['| File | Line | Status | Mutator | Original | Mutant |', '|---|---|---|---|---|---|']
    size = sum(len(r) + 1 for r in rows)
    for index, (name, line, _, status, mutator, before, after) in enumerate(found):
        row = (f'| `web/{name}` | {line} | {"no test reaches it" if status == "NoCoverage" else "survived"} '
               f'| {mutator} | {snippet(before)} | {snippet(after)} |')
        if size + len(row) + 1 > limit:
            return rows, len(found) - index
        rows.append(row)
        size += len(row) + 1
    return rows, 0


def report_text(plan_data, counts, found, limit):
    total = sum(counts.values())
    detected = sum(counts.get(s, 0) for s in DETECTED)
    valid = detected + sum(counts.get(s, 0) for s in UNDETECTED)
    score = f'{100 * detected / valid:.1f} %' if valid else 'none'
    parts = [
        f'{plan_data["label"]} at `{plan_data["sha"][:12]}`, the lines of `web/src` changed since '
        f'{plan_data["base"]}: {plural(len(plan_data["files"]), "file")}, {plural(plan_data["lines"], "line")}, '
        f'{plural(total, "mutant")}.',
        '',
        '| Killed | Timeout | Survived | No test reaches it | Other | Detected |',
        '|---|---|---|---|---|---|',
        f'| {counts.get("Killed", 0)} | {counts.get("Timeout", 0)} | {counts.get("Survived", 0)} '
        f'| {counts.get("NoCoverage", 0)} | {total - valid} | {score} |',
        '',
    ]
    if plan_data['skipped']:
        parts += [f'**Cut:** {plural(len(plan_data["skipped"]), "file")} past the limits '
                  f'({plan_data["limits"]["files"]} files, {plan_data["limits"]["lines"]} lines) '
                  'were not mutated: ' + ', '.join(f'`{s["path"]}` ({s["lines"]} lines)'
                                                  for s in plan_data['skipped']) + '.', '']
    if found:
        rows, left = table(found, limit - sum(len(p) + 1 for p in parts) - len(READING) - 200)
        parts += [f'### {plural(len(found), "mutant")} no test detected', ''] + rows
        if left:
            parts += ['', f'{left} more are in the run\'s artifact `mutation-web`.']
        parts.append('')
    else:
        parts += ['No mutant survived.', '']
    return '\n'.join(parts + [READING])


def summary(report_path, plan_path):
    plan_data = json.loads(Path(plan_path).read_text(encoding='utf-8'))
    if Path(report_path).exists():
        counts, found = undetected(json.loads(Path(report_path).read_text(encoding='utf-8')))
    elif not plan_data['files']:
        counts, found = {}, []
    else:
        raise FileNotFoundError(f'no Stryker report at {report_path}')
    text = '## Web mutation testing\n\n' + report_text(plan_data, counts, found, SUMMARY_LIMIT)
    path = os.environ.get('GITHUB_STEP_SUMMARY')
    if path:
        with open(path, 'a', encoding='utf-8') as out:
            out.write(text + '\n')
    else:
        print(text)
    Path('issue.json').write_text(json.dumps({
        'count': len(found), 'fingerprint': fingerprint(found), 'sha': plan_data['sha'],
        'body': report_text(plan_data, counts, found, ISSUE_LIMIT - 1000),
    }) + '\n', encoding='utf-8')
    output(undetected=len(found))


def decide(existing, info, run_url):
    """What `issue` does: a list of ('create', None, body), ('edit', number,
    body), ('comment', number, text) and ('close', number, None).
    `existing` is the open issue's (number, body) or None."""
    sha = info['sha'][:12]
    if info['count'] == 0:
        if existing is None:
            return []
        return [('comment', existing[0], f'No mutant survives at {sha}: {run_url}'), ('close', existing[0], None)]
    body = (f'[Scheduled run]({run_url}); the full report is its artifact `mutation-web`.\n\n'
            f'{info["body"]}\n\n{LINEAR}\n\n<!-- mutation-web: {info["fingerprint"]} -->\n')
    if existing is None:
        return [('create', None, body)]
    m = FINGERPRINT.search(existing[1] or '')
    if m and m[1] == info['fingerprint']:
        return []
    return [('edit', existing[0], body),
            ('comment', existing[0], f'{info["count"]} mutants survive at {sha}, '
                                     f'a different list than before: {run_url}. The description lists them.')]


def open_issue(repo):
    found = json.loads(run('gh', 'issue', 'list', '--repo', repo, '--state', 'open', '--limit', '100',
                           '--search', f'in:title "{TITLE}"', '--json', 'number,title,body'))
    match = [i for i in found if i['title'] == TITLE]
    return (match[0]['number'], match[0]['body']) if match else None


def issue(info_path, dry_run):
    repo = os.environ['GITHUB_REPOSITORY']
    run_url = f"{os.environ['GITHUB_SERVER_URL']}/{repo}/actions/runs/{os.environ['GITHUB_RUN_ID']}"
    info = json.loads(Path(info_path).read_text(encoding='utf-8'))
    existing = open_issue(repo)
    actions = decide(existing, info, run_url)
    print(f'Open issue "{TITLE}": {"#" + str(existing[0]) if existing else "none"}; '
          f'{info["count"]} mutants undetected.')
    if not actions:
        print('Nothing to do.')
    for action, number, text in actions:
        if dry_run:
            print(f'Would {action} {"#" + str(number) if number else "an issue"}' + (f':\n{text}\n' if text else ''))
        elif action == 'create':
            print(run('gh', 'issue', 'create', '--repo', repo, '--title', TITLE, '--body-file', '-', stdin=text))
        elif action == 'edit':
            run('gh', 'issue', 'edit', str(number), '--repo', repo, '--body-file', '-', stdin=text)
            print(f'Updated #{number}')
        elif action == 'comment':
            run('gh', 'issue', 'comment', str(number), '--repo', repo, '--body-file', '-', stdin=text)
            print(f'Commented on #{number}')
        else:
            run('gh', 'issue', 'close', str(number), '--repo', repo)
            print(f'Closed #{number}')


def main(argv):
    command, args = (argv[0], argv[1:]) if argv else ('', [])
    if command == 'target' and not args:
        target()
    elif command == 'plan' and len(args) == 2:
        plan(*args)
    elif command == 'summary' and len(args) == 2:
        summary(*args)
    elif command == 'issue' and args and set(args[1:]) <= {'--dry-run'}:
        issue(args[0], '--dry-run' in args[1:])
    else:
        print('usage: mutation_web.py target | plan BASE SHA | summary REPORT PLAN | issue ISSUE_JSON [--dry-run]',
              file=sys.stderr)
        return 2
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
