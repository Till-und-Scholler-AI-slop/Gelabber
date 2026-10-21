"""backup.sh never writes the dump or the MinIO archive in plain text when an
age recipient is set, and a failure leaves the writers stopped (#299).

A fake docker on PATH plays Compose and the two producers, and a stub age
stands in for the real one, so CI needs neither. With age installed, two
more tests make and decrypt a real backup and try malformed recipients.
"""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[2] / 'backup.sh'
RECIPIENT = 'age1fakerecipientforthebackuptest'
DUMP = (b'pgdump\n' * 40000)[:200000]
ARCHIVE = (b'minio\n' * 60000)[:300000]

# Larger than a pipe's buffer, so a reader that went away shows as EPIPE.
FAKE_DOCKER = """#!/bin/sh
printf '%s\\n' "$*" >> "$DOCKER_LOG"
for found in backups/*/database.pgdump backups/*/minio-data.tgz; do
  if [ -e "$found" ]; then
    printf 'plain %s during: %s\\n' "$found" "$*" >> "$DOCKER_VIOLATIONS"
  fi
done
case "$1 $2" in
  'compose ps') echo minio-id ;;
  'compose stop') ;;
  'compose start')
    for found in backups/*; do
      if [ ! -e "$found/SHA256SUMS" ]; then
        printf 'started before SHA256SUMS\\n' >> "$DOCKER_VIOLATIONS"
      fi
    done ;;
  'compose exec')
    yes pgdump | head -c 200000 || exit
    if [ -n "${DUMP_FAIL:-}" ]; then exit 1; fi ;;
  'inspect --format')
    case "$3" in
      *Mounts*) echo minio-volume ;;
      *Image*) echo sha256:minio-image ;;
    esac ;;
  run*)
    yes minio | head -c 300000 || exit
    if [ -n "${TAR_FAIL:-}" ]; then exit 1; fi ;;
  *) printf 'unexpected: %s\\n' "$*" >> "$DOCKER_VIOLATIONS"; exit 1 ;;
esac
"""

# Encryption only: `age -r RECIPIENT [-o OUT]`. Writes a marker line and its
# input. AGE_FAIL=late reads all input, then fails (a full disk); early fails
# without reading. The recipient check (no -o) never fails.
FAKE_AGE = """#!/bin/sh
recipient= out=
while [ "$#" -gt 0 ]; do
  case "$1" in
    -r) recipient=$2; shift 2 ;;
    -o) out=$2; shift 2 ;;
    *) echo "fake age: unexpected $1" >&2; exit 2 ;;
  esac
done
case "$recipient" in
  age1*) ;;
  *) echo "fake age: malformed recipient" >&2; exit 1 ;;
esac
if [ -n "$out" ]; then
  case "${AGE_FAIL:-}" in
    early) exit 1 ;;
    late) cat > /dev/null; exit 1 ;;
  esac
  exec > "$out"
fi
printf 'FAKE-AGE %s\\n' "$recipient"
exec cat
"""


def tool(directory, name, text):
    path = Path(directory) / name
    path.write_text(text)
    path.chmod(0o755)


class Backup(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.bin = self.tmp / 'bin'
        self.bin.mkdir()
        self.compose = self.tmp / 'compose'
        self.compose.mkdir()
        tool(self.bin, 'docker', FAKE_DOCKER)
        self.log = self.tmp / 'docker.log'
        self.violations = self.tmp / 'violations'

    def run_backup(self, recipient='', fake_age=True, **env):
        if fake_age:
            tool(self.bin, 'age', FAKE_AGE)
        environment = dict(
            os.environ, PATH=f'{self.bin}:{os.environ["PATH"]}',
            DOCKER_LOG=str(self.log), DOCKER_VIOLATIONS=str(self.violations),
            GELABBER_BACKUP_AGE_RECIPIENT=recipient, **env)
        return subprocess.run(['sh', str(SCRIPT)], cwd=self.compose, env=environment,
                              capture_output=True, timeout=60)

    def backup_dir(self):
        found = list((self.compose / 'backups').iterdir())
        self.assertEqual(len(found), 1, found)
        return found[0]

    def calls(self):
        return self.log.read_text().splitlines() if self.log.exists() else []

    def assert_no_violations(self):
        self.assertFalse(self.violations.exists(),
                         self.violations.read_text() if self.violations.exists() else '')

    def assert_sums_check(self, directory):
        check = subprocess.run(['sha256sum', '-c', 'SHA256SUMS'], cwd=directory,
                               capture_output=True, text=True)
        self.assertEqual(check.returncode, 0, check.stdout + check.stderr)

    def reset(self):
        shutil.rmtree(self.compose / 'backups', ignore_errors=True)
        self.log.unlink(missing_ok=True)
        self.violations.unlink(missing_ok=True)

    def assert_failed_with_writers_stopped(self, result):
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Backup fehlgeschlagen', result.stderr.decode())
        calls = self.calls()
        self.assertIn('compose stop api web media', calls)
        self.assertIn('compose stop minio', calls)
        self.assertFalse([call for call in calls if call.startswith('compose start')], calls)
        directory = self.backup_dir()
        self.assertFalse((directory / 'SHA256SUMS').exists())
        return directory

    def test_plain_backup(self):
        result = self.run_backup()
        self.assertEqual(result.returncode, 0, result.stderr)
        directory = self.backup_dir()
        self.assertEqual(sorted(path.name for path in directory.iterdir()),
                         ['SHA256SUMS', 'database.pgdump', 'minio-data.tgz', 'minio-image-id.txt'])
        self.assertEqual((directory / 'database.pgdump').read_bytes(), DUMP)
        self.assertEqual((directory / 'minio-data.tgz').read_bytes(), ARCHIVE)
        self.assertEqual((directory / 'minio-image-id.txt').read_text(), 'sha256:minio-image\n')
        self.assert_sums_check(directory)
        self.assertEqual(self.calls()[-1], 'compose start minio api web media')

    def test_encrypted_backup_never_holds_plain_text(self):
        result = self.run_backup(RECIPIENT)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_no_violations()
        directory = self.backup_dir()
        self.assertEqual(sorted(path.name for path in directory.iterdir()),
                         ['SHA256SUMS', 'database.pgdump.age', 'minio-data.tgz.age',
                          'minio-image-id.txt'])
        marker = f'FAKE-AGE {RECIPIENT}\n'.encode()
        self.assertEqual((directory / 'database.pgdump.age').read_bytes(), marker + DUMP)
        self.assertEqual((directory / 'minio-data.tgz.age').read_bytes(), marker + ARCHIVE)
        # Checked as stored, before decrypting and without the key.
        names = [line.split()[1] for line in (directory / 'SHA256SUMS').read_text().splitlines()]
        self.assertEqual(names, ['database.pgdump.age', 'minio-data.tgz.age', 'minio-image-id.txt'])
        self.assert_sums_check(directory)
        self.assertEqual(self.calls()[-1], 'compose start minio api web media')

    def test_failing_producer_leaves_no_final_file_and_writers_stopped(self):
        for variable, name in (('DUMP_FAIL', 'database.pgdump'), ('TAR_FAIL', 'minio-data.tgz')):
            for recipient in ('', RECIPIENT):
                with self.subTest(variable=variable, encrypted=bool(recipient)):
                    self.reset()
                    result = self.run_backup(recipient, **{variable: '1'})
                    directory = self.assert_failed_with_writers_stopped(result)
                    if not recipient:
                        # The cut off file stays as evidence.
                        self.assertTrue((directory / name).exists())
                        continue
                    self.assert_no_violations()
                    self.assertFalse((directory / name).exists())
                    # age sealed what it got: only the .part name says it is cut off.
                    self.assertFalse((directory / f'{name}.age').exists())
                    self.assertTrue((directory / f'{name}.age.part').exists())

    def test_failing_age_leaves_no_final_file_and_writers_stopped(self):
        for mode in ('late', 'early'):
            with self.subTest(mode=mode):
                self.reset()
                result = self.run_backup(RECIPIENT, AGE_FAIL=mode)
                directory = self.assert_failed_with_writers_stopped(result)
                self.assert_no_violations()
                self.assertEqual(sorted(path.name for path in directory.iterdir()),
                                 ['.database.pgdump.fifo', 'minio-image-id.txt'])
                # The dump is the first file; the archive is never started.
                self.assertFalse([call for call in self.calls() if call.startswith('run')])

    def test_a_change_to_the_script_starts_ci(self):
        workflow = (SCRIPT.parents[1] / '.github/workflows/ci.yml').read_text()
        paths = workflow.split('  pull_request:', 1)[1].split('\n\n', 1)[0]
        self.assertIn('      - deploy/backup.sh\n', paths)

    def test_malformed_recipient_stops_nothing(self):
        result = self.run_backup('not-an-age-key')
        self.assertEqual(result.returncode, 1)
        self.assertIn('not a valid age recipient. Nothing was stopped.', result.stderr.decode())
        self.assertEqual(self.calls(), [])
        self.assertFalse((self.compose / 'backups').exists())

    @unittest.skipUnless(shutil.which('age') and shutil.which('age-keygen'), 'age not installed')
    def test_real_age_round_trip(self):
        key = self.tmp / 'key.txt'
        subprocess.run(['age-keygen', '-o', str(key)], check=True, capture_output=True)
        public = subprocess.run(['age-keygen', '-y', str(key)], check=True,
                                capture_output=True, text=True).stdout.strip()
        result = self.run_backup(public, fake_age=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_no_violations()
        directory = self.backup_dir()
        self.assert_sums_check(directory)
        for name, plain in (('database.pgdump', DUMP), ('minio-data.tgz', ARCHIVE)):
            with self.subTest(name=name):
                decrypted = subprocess.run(
                    ['age', '-d', '-i', str(key), str(directory / f'{name}.age')],
                    check=True, capture_output=True).stdout
                self.assertEqual(decrypted, plain)

    @unittest.skipUnless(shutil.which('age'), 'age not installed')
    def test_real_age_refuses_malformed_recipients(self):
        for recipient in ('garbage', 'ssh-ed25519 AAAAnot-a-key'):
            with self.subTest(recipient=recipient):
                self.reset()
                result = self.run_backup(recipient, fake_age=False)
                self.assertEqual(result.returncode, 1)
                self.assertEqual(self.calls(), [])


if __name__ == '__main__':
    unittest.main()
