"""How desktop-release.yml attaches the apps to a release: python3 -m unittest
discover -s desktop/packaging/tests

Both uploads wait for the whole build of desktop-native.yml, the third-party
notices included, so no release goes out with notices that are out of date,
and the pacman repository is published only after the tarball its PKGBUILD
downloads is attached. Read without a YAML library, which a runner's python
need not have."""
from pathlib import Path
import re
import unittest

WORKFLOWS = Path(__file__).parents[3] / '.github/workflows'
WORKFLOW = WORKFLOWS / 'desktop-release.yml'
BUILD = WORKFLOWS / 'desktop-native.yml'


def jobs(text):
    """Each job's `needs`, as a set, by job id."""
    body = text.split('\njobs:\n', 1)[1]
    found = {}
    current = None
    for line in body.splitlines():
        job = re.fullmatch(r'  ([A-Za-z0-9_-]+):\s*', line)
        if job:
            current = job.group(1)
            found[current] = set()
            continue
        needs = re.fullmatch(r'    needs:\s*(.+?)\s*', line)
        if current and needs:
            value = needs.group(1)
            names = value.strip('[]').split(',') if value.startswith('[') else [value]
            found[current] = {name.strip() for name in names if name.strip()}
    return found


class ReleaseWiring(unittest.TestCase):
    def setUp(self):
        self.jobs = jobs(WORKFLOW.read_text())

    def test_parser_reads_lists_and_single_names(self):
        sample = 'on: {}\njobs:\n  a:\n    runs-on: x\n  b:\n    needs: a\n  c:\n    needs: [a, b]\n'
        self.assertEqual(jobs(sample), {'a': set(), 'b': {'a'}, 'c': {'a', 'b'}})

    def test_the_build_holds_the_notices_and_both_apps(self):
        self.assertIn('    uses: ./.github/workflows/desktop-native.yml\n', WORKFLOW.read_text())
        build = jobs(BUILD.read_text())
        for job in ('notices', 'arch-repo', 'core-windows'):
            with self.subTest(job=job):
                self.assertIn(job, build)

    def test_uploads_wait_for_the_build(self):
        for job in ('release-asset', 'release-asset-windows'):
            with self.subTest(job=job):
                self.assertIn('build', self.jobs.get(job, set()))

    def test_the_repository_is_published_after_the_tarball(self):
        self.assertIn('release-asset', self.jobs.get('arch-repo-publish', set()))


if __name__ == '__main__':
    unittest.main()
