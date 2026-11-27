"""When the minio job builds MinIO and mc from source, and when it checks the
published pinned images instead (GEL-392). If the switch breaks, a pull
request that breaks a MinIO Dockerfile is checked against the old image."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[3]
WORKFLOWS = ROOT / '.github/workflows'
NOT_FOUND = "steps.published.outputs.found != 'true'"


def load(name):
    return yaml.safe_load((WORKFLOWS / name).read_text())


class MinioImages(unittest.TestCase):
    def setUp(self):
        self.images = load('minio-images.yml')
        self.steps = {step['name']: step for step in self.images['jobs']['build']['steps']}

    def test_ci_passes_minio_build_on(self):
        job = load('ci.yml')['jobs']['minio']
        self.assertEqual(job['uses'], './.github/workflows/minio-images.yml')
        self.assertEqual(job['with']['build'], "${{ needs.changes.outputs.minio-build == 'true' }}")
        self.assertIn('build', self.images[True]['workflow_call']['inputs'])

    def test_the_published_images_are_pulled_only_when_no_build_is_asked(self):
        self.assertEqual(self.steps['Published images']['if'], '${{ !inputs.build }}')

    def test_everything_that_builds_or_keeps_runs_unless_the_images_were_pulled(self):
        for name in ('Set up Buildx', 'Build', 'Build mc', 'Save the images', 'Upload the images'):
            with self.subTest(step=name):
                self.assertIn(NOT_FOUND, str(self.steps[name].get('if')))
        self.assertEqual(self.images['jobs']['build']['outputs']['built'], f'${{{{ {NOT_FOUND} }}}}')

    def test_the_checks_run_on_either_image(self):
        for name in ('Compose pin matches', 'Verify minio --version', 'Verify mc --version',
                     'minio-init grants the bucket only'):
            with self.subTest(step=name):
                self.assertNotIn('if', self.steps[name])



def step(workflow, job, name):
    return next(step for step in load(workflow)['jobs'][job]['steps'] if step.get('name') == name)


def run(script, env, tools):
    """A run: script with fake tools on PATH; returns its exit code and the
    calls the tools logged."""
    with tempfile.TemporaryDirectory() as tmp:
        log = Path(tmp) / 'calls'
        for name, body in tools.items():
            tool = Path(tmp) / name
            tool.write_text('#!/bin/sh\necho "$0 $*" >> "$CALLS"\n' + body)
            tool.chmod(0o755)
        output = Path(tmp) / 'output'
        result = subprocess.run(['bash', '-c', script], capture_output=True, text=True,
                                env={**os.environ, **env, 'CALLS': str(log), 'RUNNER_TEMP': tmp,
                                     'GITHUB_OUTPUT': str(output),
                                     'PATH': f'{tmp}{os.pathsep}{os.environ["PATH"]}'})
        calls = log.read_text().splitlines() if log.exists() else []
        out = dict(line.split('=', 1) for line in output.read_text().splitlines()) if output.exists() else {}
    return result.returncode, calls, out


class Publish(unittest.TestCase):
    """GEL-402: a changed Dockerfile replaces its image's tag on main; a run
    that changed and built nothing pushes nothing."""

    def changed(self, event, paths):
        _, _, out = run(step('publish-minio.yml', 'changed', 'Diff since the previous head')['run'],
                        {'EVENT': event, 'BEFORE': 'a' * 40, 'FAKE_DIFF': '\n'.join(paths)},
                        {'git': 'case "$1" in diff) printf "%s\\n" "$FAKE_DIFF" ;; esac\n'})
        return out

    def test_a_changed_dockerfile_asks_for_a_rebuild_of_that_image(self):
        self.assertEqual(self.changed('push', ['deploy/compose/minio/Dockerfile']), {'minio': 'true', 'mc': 'false'})
        self.assertEqual(self.changed('push', ['deploy/compose/mc/Dockerfile']), {'minio': 'false', 'mc': 'true'})
        self.assertEqual(self.changed('push', ['.github/workflows/publish-minio.yml']),
                         {'minio': 'false', 'mc': 'false'})
        self.assertEqual(self.changed('workflow_dispatch', ['deploy/compose/minio/Dockerfile']),
                         {'minio': 'false', 'mc': 'false'})
        jobs = load('publish-minio.yml')['jobs']
        self.assertEqual(jobs['images']['with']['build'],
                         "${{ needs.changed.outputs.minio == 'true' || needs.changed.outputs.mc == 'true' }}")
        self.assertEqual(jobs['publish']['env']['REPLACE_MINIO'], '${{ needs.changed.outputs.minio }}')
        self.assertEqual(jobs['publish']['env']['REPLACE_MC'], '${{ needs.changed.outputs.mc }}')

    def push(self, existing, replace_minio='false', replace_mc='false', inspect_error='not found'):
        docker = ('case "$1 $2 $3" in "buildx imagetools inspect")\n'
                  '  for ref in $EXISTING; do [ "$ref" = "$4" ] && exit 0; done\n'
                  f'  [ "{inspect_error}" = "not found" ] && echo "ERROR: $4: not found" >&2 || echo "unauthorized" >&2\n'
                  '  exit 1 ;; esac\n')
        code, calls, _ = run(step('publish-minio.yml', 'publish', 'Push to GHCR')['run'],
                             {'GHCR_IMAGE': 'ghcr.io/o/r/minio', 'LOCAL_IMAGE': 'gelabber/minio',
                              'GHCR_MC_IMAGE': 'ghcr.io/o/r/mc', 'LOCAL_MC_IMAGE': 'gelabber/mc',
                              'MINIO_RELEASE': 'R1', 'MC_RELEASE': 'R2', 'EXISTING': ' '.join(existing),
                              'REPLACE_MINIO': replace_minio, 'REPLACE_MC': replace_mc},
                             {'docker': docker})
        return code, [call.split()[-1] for call in calls if call.split()[1] == 'push']

    def test_a_missing_tag_is_pushed(self):
        self.assertEqual(self.push([]), (0, ['ghcr.io/o/r/minio:R1', 'ghcr.io/o/r/mc:R2']))

    def test_an_existing_tag_is_kept_unless_its_dockerfile_changed(self):
        both = ['ghcr.io/o/r/minio:R1', 'ghcr.io/o/r/mc:R2']
        self.assertEqual(self.push(both), (0, []))
        self.assertEqual(self.push(both, replace_minio='true'), (0, ['ghcr.io/o/r/minio:R1']))
        self.assertEqual(self.push(both, replace_mc='true'), (0, ['ghcr.io/o/r/mc:R2']))

    def test_a_registry_error_pushes_nothing(self):
        code, pushed = self.push([], inspect_error='unauthorized')
        self.assertNotEqual(code, 0)
        self.assertEqual(pushed, [])

    def test_a_run_that_built_nothing_pushes_nothing(self):
        self.assertIn("needs.images.outputs.built == 'true'", load('publish-minio.yml')['jobs']['publish']['if'])

if __name__ == '__main__':
    unittest.main()
