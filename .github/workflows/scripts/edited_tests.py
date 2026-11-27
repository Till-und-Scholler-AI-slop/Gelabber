"""Test edits a reviewer should look at before merging (GEL-387).

ci.yml's changes job runs this over a pull request's diff. It lists, in the
job summary and as warning annotations:

- deleted tests, and assertions deleted from tests that stay,
- assertions made looser (exact to "contains", "one of", truthy or a range),
- new skips (`#[ignore]`, `.skip`, `.todo`, `skipIf`, ...) and early returns,
- raised limits and timeouts, added sleeps, waits and retry loops,
- changes to the test runners' configuration.

It reads diff lines with patterns, so it can miss a rewrite and point at a
sound change; it says where to look. It never fails the run.

Usage: edited_tests.py BASE HEAD
"""
import difflib
import os
import re
import subprocess
import sys
from dataclasses import dataclass, field

TEST_PATH = re.compile(
    r'(?:^|/)(?:tests?|e2e|__tests__)/'
    r'|\.(?:test|spec)\.[cm]?[jt]sx?$'
    r'|(?:^|/)test_[^/]*\.py$|_test\.py$|(?:^|/)conftest\.py$'
    r'|(?:^|/)smoke-[^/]*\.mjs$|\.checks\.mjs$')
RUNNER_CONFIG = re.compile(
    r'(?:^|/)(?:vitest|vite|playwright|jest)\.config\.[cm]?[jt]s$'
    r'|(?:^|/)\.config/nextest\.toml$|(?:^|/)nextest\.toml$'
    r'|(?:^|/)pytest\.ini$|(?:^|/)setup\.cfg$|(?:^|/)tox\.ini$')
# A package.json line that names a test script or a test runner.
RUNNER_SCRIPT = re.compile(r'"test"\s*:')

ASSERTION = {
    'rs': re.compile(r'\b(?:debug_)?assert(?:_eq|_ne|_matches)?!\s*\(|\.expect_err\(|\bassert_\w+\('),
    'py': re.compile(r'^\s*assert\b|\bself\.assert\w+\(|\bpytest\.raises\b'),
    'js': re.compile(r'\bexpect(?:\.\w+)?\s*\(|\bassert(?:\.\w+)?\s*\('),
}
STRICT = {
    'rs': re.compile(r'\bassert_(?:eq|ne)!\s*\('),
    'py': re.compile(r'^\s*assert\s+.*==|assert(?:Equal|DictEqual|ListEqual|CountEqual|Is)\('),
    'js': re.compile(r'\.(?:toBe|toEqual|toStrictEqual|toHaveBeenCalledWith|toHaveBeenCalledTimes|'
                     r'toHaveLength|toHaveBeenNthCalledWith|toHaveTextContent|toHaveAttribute|toHaveValue|'
                     r'toMatchInlineSnapshot|toBeNull|toBeUndefined)\(|'
                     r'assert\.(?:equal|strictEqual|deepEqual|deepStrictEqual)\('),
}
LOOSE = {
    'rs': re.compile(r'\bassert!\s*\(|\bassert_matches!|\bmatches!\s*\(|\.contains\(|\.starts_with\(|'
                     r'\.ends_with\(|\.any\(|\.is_some\(\)|\.is_ok\(\)|\.is_err\(\)'),
    'py': re.compile(r'^\s*assert\s+.*\bin\b|assert(?:In|True|Regex|Greater|Less|IsNotNone)\w*\('),
    'js': re.compile(r'\.(?:toMatchObject|toContain|toContainEqual|toMatch|toBeTruthy|toBeFalsy|toBeDefined|'
                     r'toBeGreaterThan|toBeGreaterThanOrEqual|toBeLessThan|toBeLessThanOrEqual|toBeCloseTo|'
                     r'toHaveBeenCalled|toHaveProperty|toBeInstanceOf)\(|objectContaining|stringContaining|'
                     r'arrayContaining|stringMatching|expect\.any(?:thing)?\(|assert\.(?:ok|match)\('),
}
# "One of several values": a pattern with alternatives, a list it must be in.
ONE_OF = re.compile(r'matches!\s*\([^,]+,[^)]*\|'
                    r'|\]\s*\)?\s*\.includes\('
                    r'|\bin\s*[\[(\{][^\])}]*,'
                    r'|\boneOf\b|\bany_of\b')
SKIP = re.compile(r'#\[ignore\b|#\[cfg_attr\([^)]*\bignore\b'
                  r'|\b(?:it|test|describe|suite)\.(?:skip|todo|skipIf|runIf|fixme|fails)\b'
                  r'|\bx(?:it|describe|test)\s*\('
                  r'|pytest\.mark\.(?:skip|skipif|xfail)|\bpytest\.skip\(|@unittest\.skip|\bskipTest\('
                  r'|\btest\.skip\(|\bctx\.skip\(|\.skip\(\s*\)')
EARLY_RETURN = re.compile(r'^\s*(?:if\b.*[){]\s*)?return\s*;?\s*\}?\s*$')
SLEEP = re.compile(r'thread::sleep|time::sleep\(|\bsleep\s*\(|\bsetTimeout\s*\(|\bwaitForTimeout\s*\(|'
                   r'\btime\.sleep\(|\bAtomics\.wait\(|\bawait\s+(?:delay|wait|pause)\s*\(|\bsleep\s+\d')
# Polling until an expectation holds (a locator's plain waitFor() is not).
WAIT = re.compile(r'\.toPass\(|\bexpect\.poll\(|\bwaitFor\s*\(\s*(?:async\s*)?\(|\beventually\b')
RETRY = re.compile(r'\bretr(?:y|ies|ying)\w*\s*\(|\bretries\s*[:=]\s*[1-9]|'
                   r'^\s*while\b.*\b(?:is_none|is_empty|is_err)\(\)|^\s*loop\s*\{|'
                   r'^\s*for\s+(?:_\w*|attempt\w*)\s+in\s+\d+\s*\.\.|'
                   r'^\s*for\s*\(\s*let\s+(?:attempt|tries|retry)\w*\b')
TIMEOUT = re.compile(r'(?i)\btimeout\w*\s*[:=(]|\btestTimeout\b|\bhookTimeout\b|\.timeout\(|#\[timeout')
TIME_WORD = re.compile(r'(?i)timeout|wait|deadline|duration|from_secs|from_millis|within|interval|attempt|'
                       r'retr|tries|poll|sleep|delay|budget|_ms\b|_secs\b|\bms\b|seconds|millis|grace|'
                       r'toBeLessThan|toBeGreaterThan|tolerance|epsilon|threshold|limit|max_|min_|\s[<>]=?\s')
NUMBER = re.compile(r'(?<![\w.])\d[\d_]*(?:\.\d+)?(?![\w.])')
COMMENT = re.compile(r'^\s*(?://|/\*|\*|#(?!\[)|--)')

TEST_DECL = {
    'rs': re.compile(r'^\s*#\[(?:[\w:]+::)?test\b'),
    'py': re.compile(r'^\s*(?:async\s+)?def\s+test_\w+'),
    'js': re.compile(r'^\s*(?:it|test)(?:\.\w+(?:\([^)]*\))?)*\s*[(`]'),
}
TEST_NAME = {
    'rs': re.compile(r'^\s*(?:pub(?:\([\w ]+\))?\s+)?(?:async\s+)?fn\s+(\w+)'),
    'py': re.compile(r'^\s*(?:async\s+)?def\s+(test_\w+)'),
    'js': re.compile(r'^\s*(?:it|test)(?:\.\w+(?:\([^)]*\))?)*\s*\(\s*([\'"`])(.*?)\1'),
}
LIMIT = 50


@dataclass
class Line:
    sign: str  # '-', '+' or ' '
    text: str
    old: int  # line number in the base file, 0 for an added line
    new: int  # line number in the head file, 0 for a removed line


@dataclass
class File:
    old_path: str
    new_path: str
    lines: list = field(default_factory=list)
    hunks: list = field(default_factory=list)  # lists of Line

    @property
    def path(self):
        return self.new_path if self.new_path != '/dev/null' else self.old_path


@dataclass
class Finding:
    path: str
    line: int
    kind: str
    detail: str


def parse(diff):
    """The files of a unified diff (git diff --no-renames), with their hunks."""
    files, current, old, new = [], None, 0, 0
    for raw in diff.splitlines():
        if raw.startswith('diff --git '):
            current = File('', '')
            files.append(current)
        elif current is None:
            continue
        elif raw.startswith('--- '):
            current.old_path = raw[4:].removeprefix('a/')
        elif raw.startswith('+++ '):
            current.new_path = raw[4:].removeprefix('b/')
        elif raw.startswith('@@'):
            match = re.match(r'@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@', raw)
            old, new = int(match.group(1)), int(match.group(2))
            current.hunks.append([])
        elif current.hunks and raw[:1] in ('-', '+', ' '):
            sign, text = raw[0], raw[1:]
            line = Line(sign, text, old if sign != '+' else 0, new if sign != '-' else 0)
            old += sign != '+'
            new += sign != '-'
            current.hunks[-1].append(line)
            current.lines.append(line)
    return files


def code(text):
    """A line with its string literals emptied: a skip or sleep named in a
    string or a docstring is no skip or sleep."""
    return re.sub(r'"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])*\'|`(?:\\.|[^`\\])*`', '""', text)


def language(path):
    if path.endswith('.rs'):
        return 'rs'
    if path.endswith('.py'):
        return 'py'
    if re.search(r'\.[cm]?[jt]sx?$', path):
        return 'js'
    return None


def test_start(text):
    """The first line of a Rust file's test module, or None."""
    for number, line in enumerate(text.splitlines(), 1):
        if re.match(r'\s*#\[cfg\(test\)\]', line):
            return number
    return None


def in_test(file, line, read):
    """Whether a diff line is test code: a test file, or a Rust file's
    lines from its #[cfg(test)] module on."""
    path = file.old_path if line.sign == '-' else file.new_path
    if TEST_PATH.search(path):
        return True
    if not path.endswith('.rs'):
        return False
    start = read('-' if line.sign == '-' else '+', path)
    return start is not None and (line.old if line.sign == '-' else line.new) >= start


def statements(lines):
    """Consecutive lines joined until their parentheses and brackets close,
    with the first line. Braces do not join: a block is many statements."""
    out, text, first, depth = [], '', None, 0
    for line in lines:
        if COMMENT.match(line.text) and not text:
            continue
        if not text:
            first = line
        text += ' ' + line.text.strip()
        depth += sum(line.text.count(c) for c in '([') - sum(line.text.count(c) for c in ')]')
        if depth <= 0:
            out.append((first, text.strip()))
            text, depth = '', 0
    if text:
        out.append((first, text.strip()))
    return out


def blocks(hunk):
    """The runs of removed and added lines in a hunk."""
    out, removed, added = [], [], []
    for line in hunk + [Line(' ', '', 0, 0)]:
        if line.sign == '-':
            removed.append(line)
        elif line.sign == '+':
            added.append(line)
        elif removed or added:
            out.append((removed, added))
            removed, added = [], []
    return out


def similar(a, b):
    return difflib.SequenceMatcher(None, re.sub(r'\s+', '', a), re.sub(r'\s+', '', b)).ratio()


def numbers(text):
    return [float(n.replace('_', '')) for n in NUMBER.findall(text)]


def test_names(lines, lang):
    """The tests declared in these lines (a Rust test's name is the fn after
    its attribute)."""
    names, pending = [], False
    for line in lines:
        if lang == 'rs':
            if TEST_DECL['rs'].match(line.text):
                pending = True
                continue
            match = TEST_NAME['rs'].match(line.text)
            if pending and match:
                names.append((line, match.group(1)))
                pending = False
        else:
            match = TEST_NAME[lang].match(line.text)
            if match:
                names.append((line, match.groups()[-1]))
    return names


def check_file(file, read, removed_names, added_names):
    path, lang = file.path, language(file.path)
    found = []
    scripts = {sign: {line.text.strip().rstrip(',') for line in file.lines
                      if line.sign == sign and RUNNER_SCRIPT.search(line.text)} for sign in '-+'}
    if RUNNER_CONFIG.search(path) or (path.endswith('package.json') and scripts['-'] != scripts['+']):
        first = next((line for line in file.lines if line.sign != ' '), None)
        found.append(Finding(path, first.new or first.old if first else 1, 'runner configuration',
                             'the test runner\'s configuration changed'))
    if lang is None:
        return found
    created, deleted = file.old_path == '/dev/null', file.new_path == '/dev/null'
    for hunk in file.hunks:
        test_lines = [line for line in hunk if line.sign != ' ' and in_test(file, line, read)]
        if not test_lines:
            continue
        added = [line for line in test_lines if line.sign == '+']
        removed = [line for line in test_lines if line.sign == '-']
        new_test = bool(test_names(added, lang)) or any(TEST_DECL[lang].match(line.text) for line in added)
        gone = [code(old.text) for old in removed]
        for line in added:
            text = code(line.text)
            if COMMENT.match(text):
                continue
            if SKIP.search(text) and not any(SKIP.search(old) for old in gone):
                found.append(Finding(path, line.new, 'skip added', line.text.strip()))
            elif created or new_test:
                continue
            elif EARLY_RETURN.match(text):
                found.append(Finding(path, line.new, 'early return', line.text.strip()))
            elif SLEEP.search(text) and not any(SLEEP.search(old) for old in gone):
                found.append(Finding(path, line.new, 'sleep added', line.text.strip()))
            elif RETRY.search(text) and not any(RETRY.search(old) for old in gone):
                found.append(Finding(path, line.new, 'retry loop added', line.text.strip()))
            elif WAIT.search(text) and not any(WAIT.search(old) for old in gone):
                found.append(Finding(path, line.new, 'wait added', line.text.strip()))
            elif TIMEOUT.search(text) and not any(TIMEOUT.search(old) for old in gone):
                found.append(Finding(path, line.new, 'timeout added', line.text.strip()))
        if created or deleted:
            continue
        for gone, came in blocks([line for line in hunk if line.sign == ' ' or line in test_lines]):
            found += check_block(path, lang, gone, came, removed_names)
    return found


def check_block(path, lang, gone, came, removed_names):
    """Loosened or removed assertions and raised limits in one run of
    removed and added lines."""
    found = []
    assertion, strict, loose = ASSERTION[lang], STRICT[lang], LOOSE[lang]
    old = [(line, text) for line, text in statements(gone)]
    new = [(line, text) for line, text in statements(came)]
    old_asserts = [(line, text) for line, text in old if assertion.search(text)]
    new_asserts = [(line, text) for line, text in new if assertion.search(text)]
    inside_deleted_test = {name for _, name in test_names(gone, lang)} & removed_names
    # Loosened: a strict assertion gives way to a loose one in the same place.
    loosened = set()
    if len(gone) <= 40:
        for line, text in old_asserts:
            if not strict.search(text):
                continue
            for other_line, other in new_asserts:
                looser = ((loose.search(other) and not strict.search(other))
                          or (ONE_OF.search(other) and not ONE_OF.search(text)))
                if looser and (similar(text, other) >= 0.4 or len(old_asserts) == 1):
                    found.append(Finding(path, other_line.new, 'assertion loosened',
                                         f'{text[:120]} → {other[:120]}'))
                    loosened.add(text)
                    break
    # Removed: an assertion with no similar one in its place, in a test that stays.
    if not inside_deleted_test and len(gone) <= 200:
        for line, text in old_asserts:
            if text in loosened or any(similar(text, other) >= 0.6 for _, other in new_asserts):
                continue
            found.append(Finding(path, line.old, 'assertion removed', text[:160]))
    # Raised: a line that differs only in a number, which went up.
    if len(gone) <= 60:
        used = set()
        for line, text in old:
            if not NUMBER.search(text) or not TIME_WORD.search(text):
                continue
            key = NUMBER.sub('N', re.sub(r'\s+', '', text))
            for index, (other_line, other) in enumerate(new):
                if index in used or NUMBER.sub('N', re.sub(r'\s+', '', other)) != key:
                    continue
                before, after = numbers(text), numbers(other)
                if len(before) == len(after) and any(b > a for a, b in zip(before, after)) \
                        and not any(b < a for a, b in zip(before, after)):
                    used.add(index)
                    found.append(Finding(path, other_line.new, 'limit raised',
                                         f'{text[:110]} → {other[:110]}'))
                    break
    return found


def check(diff, read):
    """Findings for a diff; `read(side, path)` gives a Rust file's first
    #[cfg(test)] line on the base ('-') or head ('+') side."""
    files = parse(diff)
    removed_names, added_names = set(), set()
    for file in files:
        lang = language(file.path)
        if lang is None:
            continue
        test = [line for line in file.lines if line.sign != ' ' and in_test(file, line, read)]
        removed_names |= {name for _, name in test_names([line for line in test if line.sign == '-'], lang)}
        added_names |= {name for _, name in test_names([line for line in test if line.sign == '+'], lang)}
    deleted = removed_names - added_names
    found = []
    for file in files:
        lang = language(file.path)
        if lang:
            gone = [(line, name) for line, name in test_names(
                [line for line in file.lines if line.sign == '-' and in_test(file, line, read)], lang)
                if name in deleted]
            if gone and file.new_path == '/dev/null':
                found.append(Finding(file.path, 1, 'test file deleted',
                                     f'{len(gone)} tests: ' + ', '.join(name for _, name in gone)[:300]))
            else:
                found += [Finding(file.path, line.old, 'test deleted', name) for line, name in gone]
        found += check_file(file, read, deleted, added_names)
    return found


def git_reader(base, head):
    cache = {}

    def read(side, path):
        key = (side, path)
        if key not in cache:
            try:
                text = subprocess.run(['git', 'show', f'{base if side == "-" else head}:{path}'],
                                      capture_output=True, text=True, check=True).stdout
            except subprocess.CalledProcessError:
                text = ''
            cache[key] = test_start(text)
        return cache[key]
    return read


def escape(text):
    return text.replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')


def report(found):
    """Annotations for stdout and the job summary's Markdown."""
    annotations = [f'::warning file={f.path},line={max(f.line, 1)},title=Test change: {f.kind}::'
                   f'{escape(f.detail)}' for f in found[:LIMIT]]
    if not found:
        return annotations, 'No test edits to look at: no deleted or loosened assertions, skips, ' \
                            'raised limits, sleeps, retries or runner changes.\n'
    rows = '\n'.join('| `{}:{}` | {} | `{}` |'.format(f.path, f.line, f.kind, f.detail[:200].replace('|', '\\|'))
                     for f in found)
    return annotations, ('### Test edits to look at\n\n'
                         'These edits can make a failing test pass. Check that each one is meant; '
                         'this list does not fail the run.\n\n'
                         '| Where | What | Line |\n|---|---|---|\n' + rows + '\n')


def main(argv):
    base, head = argv
    diff = subprocess.run(['git', '-c', 'core.quotePath=false', 'diff', '--no-renames', '--no-color',
                           '-U3', base, head], capture_output=True, text=True, check=True).stdout
    found = check(diff, git_reader(base, head))
    annotations, summary = report(found)
    print('\n'.join(annotations))
    summary_path = os.environ.get('GITHUB_STEP_SUMMARY')
    if summary_path:
        with open(summary_path, 'a', encoding='utf-8') as out:
            out.write(summary)
    else:
        print(summary)


if __name__ == '__main__':
    try:
        main(sys.argv[1:])
    except Exception as error:  # noqa: BLE001 - a report, never a gate
        print(f'::warning title=Test edits::the report failed: {error}')
