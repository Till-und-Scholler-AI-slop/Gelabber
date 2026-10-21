"""Every action a workflow runs is pinned to a commit SHA (#299).

A tag or branch can be moved to other code at any time, also to code that
reads the secrets of the job it runs in (the GHCR token, the updater's
signing key). A pin is the full commit SHA with the version it was resolved
from as a comment, the form Dependabot (.github/dependabot.yml) updates:

    uses: actions/checkout@<40 hex digits> # v7.0.1

Local actions (./...) and Docker images by digest need no pin. Read as text,
without a YAML library. workflow-pins.yml runs this on every change to a
workflow; ci.yml runs it with the other deployment tests.
"""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[3]
WORKFLOWS = ROOT / '.github/workflows'
USES = re.compile(r'\s*(?:-\s+)?uses:\s*(.*?)\s*')
PINNED = re.compile(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+(/[^@\s]+)?@[0-9a-f]{40}\s+#\s*\S+')
DOCKER = re.compile(r'docker://[^@\s]+@sha256:[0-9a-f]{64}(\s.*)?')


def unpinned(name, text):
    """`name:line: ref` for each `uses:` of a workflow's text that is not
    pinned."""
    found = []
    for number, line in enumerate(text.splitlines(), 1):
        uses = USES.fullmatch(line)
        if not uses:
            continue
        ref = uses.group(1)
        if ref.startswith('./') or DOCKER.fullmatch(ref) or PINNED.match(ref):
            continue
        found.append(f'{name}:{number}: {ref}')
    return found


def workflows():
    return sorted([*WORKFLOWS.glob('*.yml'), *WORKFLOWS.glob('*.yaml')])


class WorkflowPins(unittest.TestCase):
    def test_check_rejects_tags_branches_and_bare_shas(self):
        sha = '3d3c42e5aac5ba805825da76410c181273ba90b1'
        sample = '\n'.join([
            'jobs:',
            '  a:',
            '    steps:',
            '      - uses: actions/checkout@v7',
            '      - name: Cache',
            '        uses: actions/cache/restore@v4',
            f'      - uses: actions/checkout@{sha}',
            f'      - uses: actions/checkout@{sha[:12]} # v7.0.1',
            '      - uses: dtolnay/rust-toolchain@1.98.1 # a branch',
            '      - uses: docker://alpine:3.22',
            f'      - uses: actions/checkout@{sha} # v7.0.1',
            f'        uses: actions/cache/save@{sha} # v4.3.0',
            f'      - uses: dtolnay/rust-toolchain@{sha} # 1.98.1 (branch)',
            '      - uses: ./.github/actions/local',
            f'      - uses: docker://alpine@sha256:{"0" * 64}',
            '      # uses: actions/checkout@v7',
            '      - run: echo done',
        ])
        self.assertEqual(unpinned('sample.yml', sample), [
            'sample.yml:4: actions/checkout@v7',
            'sample.yml:6: actions/cache/restore@v4',
            f'sample.yml:7: actions/checkout@{sha}',
            f'sample.yml:8: actions/checkout@{sha[:12]} # v7.0.1',
            'sample.yml:9: dtolnay/rust-toolchain@1.98.1 # a branch',
            'sample.yml:10: docker://alpine:3.22',
        ])

    def test_every_action_is_pinned_to_a_commit(self):
        files = workflows()
        self.assertTrue(files, f'no workflows in {WORKFLOWS}')
        uses = sum(1 for path in files for line in path.read_text().splitlines()
                   if USES.fullmatch(line))
        self.assertGreater(uses, 0, 'the check found no `uses:` at all')
        found = [entry for path in files
                 for entry in unpinned(path.relative_to(ROOT).as_posix(), path.read_text())]
        self.assertEqual(found, [], 'Pin these to a commit SHA with the version as a comment, '
                         '`uses: owner/repo@<sha> # vX.Y.Z` (git ls-remote, peeled ^{} '
                         'for an annotated tag):\n' + '\n'.join(found))


if __name__ == '__main__':
    unittest.main()
