"""The upgrade test's boundary: which release a server upgrades from, by
the event's refs (.github/workflows/scripts/previous_release.sh, GEL-413).

Each case runs the script in a clone of a scratch repository whose origin
holds the releases v0.13.0, v0.13.1 (migrations 1 to 3) and v0.14.0
(migration 4), the branches main and v0.14 at v0.14.0, and v0.15 one
migration further while its Cargo.toml still says 0.14.0.
"""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / '.github/workflows/scripts/previous_release.sh'
SCRIPT_PATH = '.github/workflows/scripts/previous_release.sh'
MIGRATIONS = 'backend/api/migrations'


def git(cwd, *args):
    return subprocess.run(
        ['git', '-c', 'user.name=t', '-c', 'user.email=t@example.test', *args],
        cwd=cwd, check=True, capture_output=True, text=True,
    ).stdout


def commit(repo, version, migrations, message):
    (repo / 'backend').mkdir(exist_ok=True)
    (repo / 'backend/Cargo.toml').write_text(f'[workspace.package]\nversion = "{version}"\n')
    folder = repo / MIGRATIONS
    folder.mkdir(parents=True, exist_ok=True)
    for number in migrations:
        (folder / f'{number:04d}_step.sql').write_text(f'-- {number}\n')
    script = repo / SCRIPT_PATH
    script.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy(SCRIPT, script)
    git(repo, 'add', '-A')
    git(repo, 'commit', '-qm', message)


class PreviousRelease(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        base = Path(cls.tmp.name)
        origin = base / 'origin.git'
        git(base, 'init', '-q', '--bare', str(origin))
        work = base / 'work'
        git(base, 'init', '-q', '-b', 'main', str(work))
        git(work, 'remote', 'add', 'origin', str(origin))
        commit(work, '0.13.0', [1, 2], 'v0.13.0')
        git(work, 'tag', 'v0.13.0')
        commit(work, '0.13.1', [3], 'v0.13.1')
        git(work, 'tag', 'v0.13.1')
        commit(work, '0.14.0', [4], 'v0.14.0')
        git(work, 'tag', 'v0.14.0')
        git(work, 'branch', 'v0.14')
        git(work, 'switch', '-qc', 'v0.15')
        commit(work, '0.14.0', [5], 'on v0.15')
        git(work, 'push', '-q', 'origin', 'main', 'v0.14', 'v0.15', '--tags')
        cls.full = base / 'full'
        git(base, 'clone', '-q', str(origin), str(cls.full))
        git(cls.full, 'switch', '-q', 'v0.15')
        cls.origin = origin
        cls.base = base

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def run_script(self, checkout, *args, base_ref='', ref_name=''):
        env = {**os.environ, 'BASE_REF': base_ref, 'REF_NAME': ref_name}
        return subprocess.run(
            ['sh', str(checkout / SCRIPT_PATH), *args],
            cwd=self.base, env=env, capture_output=True, text=True,
        )

    def boundary(self, base_ref='', ref_name=''):
        done = self.run_script(self.full, base_ref=base_ref, ref_name=ref_name)
        self.assertEqual(done.returncode, 0, done.stderr)
        return done.stdout.strip()

    def test_each_event_upgrades_from_the_previous_minor_of_its_line(self):
        cases = [
            ('pull request to v0.14', 'v0.14', '7/merge', 'v0.13.1 3'),
            ('pull request to v0.15, Cargo still 0.14.0', 'v0.15', '8/merge', 'v0.14.0 4'),
            ('release pull request to main', 'main', '9/merge', 'v0.13.1 3'),
            ('hotfix pull request to main', 'main', '10/merge', 'v0.13.1 3'),
            ('push to main', '', 'main', 'v0.13.1 3'),
            ('push to v0.14', '', 'v0.14', 'v0.13.1 3'),
            ('push to v0.15', '', 'v0.15', 'v0.14.0 4'),
            ('merge queue on v0.14', '', 'gh-readonly-queue/v0.14/pr-7-abc', 'v0.13.1 3'),
            ('merge queue on v0.15', '', 'gh-readonly-queue/v0.15/pr-8-abc', 'v0.14.0 4'),
            ('run by hand on a feature branch', '', 'fix/gel-1-thing', 'v0.14.0 4'),
            ('run by hand on a release branch', '', 'release/v0.14.1', 'v0.13.1 3'),
            ('no refs, checked out v0.15', '', '', 'v0.14.0 4'),
        ]
        for name, base_ref, ref_name, expected in cases:
            with self.subTest(name):
                self.assertEqual(self.boundary(base_ref, ref_name), expected)

    def test_a_shallow_checkout_needs_the_tags_fetched(self):
        shallow = self.base / 'shallow'
        git(self.base, 'clone', '-q', '--depth=1', '--no-tags', '--branch', 'v0.14',
            f'file://{self.origin}', str(shallow))
        without = self.run_script(shallow, ref_name='v0.14')
        self.assertEqual(without.returncode, 1)
        self.assertIn('no tag v0.13.* in this checkout', without.stderr)
        fetched = self.run_script(shallow, '--fetch', ref_name='v0.14')
        self.assertEqual(fetched.returncode, 0, fetched.stderr)
        self.assertEqual(fetched.stdout.strip(), 'v0.13.1 3')

    def test_a_line_whose_previous_minor_has_no_release_says_so(self):
        done = self.run_script(self.full, '--fetch', ref_name='v0.16')
        self.assertEqual(done.returncode, 1)
        self.assertIn('v0.16 follows v0.15, which has no release yet', done.stderr)

    def test_the_check_refuses_changed_and_inserted_released_migrations(self):
        repo = self.base / 'check'
        git(self.base, 'clone', '-q', str(self.origin), str(repo))
        git(repo, 'switch', '-q', 'v0.14')

        def check():
            return self.run_script(repo, '--check', 'v0.13.1', '3')

        self.assertEqual(check().returncode, 0, check().stderr)
        later = repo / MIGRATIONS / '0006_later.sql'
        later.write_text('-- later\n')
        git(repo, 'add', '-A')
        git(repo, 'commit', '-qm', 'a later migration')
        self.assertEqual(check().returncode, 0, check().stderr)

        inserted = repo / MIGRATIONS / '0002_inserted.sql'
        inserted.write_text('-- inserted\n')
        git(repo, 'add', '-A')
        git(repo, 'commit', '-qm', 'inserted below the boundary')
        done = check()
        self.assertEqual(done.returncode, 1)
        self.assertIn('0002_inserted.sql', done.stderr)
        git(repo, 'revert', '--no-edit', 'HEAD')

        (repo / MIGRATIONS / '0001_step.sql').write_text('-- edited\n')
        git(repo, 'commit', '-qam', 'edited a released migration')
        done = check()
        self.assertEqual(done.returncode, 1)
        self.assertIn('0001_step.sql', done.stderr)


if __name__ == '__main__':
    unittest.main()
