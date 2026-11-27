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


class SourceLabel(unittest.TestCase):
    """Every build carries the tree its tag is compared with (GEL-402)."""

    def test_each_build_is_labelled_with_its_folders_tree(self):
        steps = {step['name']: step for step in load('minio-images.yml')['jobs']['build']['steps']}
        self.assertEqual(steps['Sources']['if'], NOT_FOUND)
        for build, name in (('Build', 'minio'), ('Build mc', 'mc')):
            with self.subTest(build=build):
                self.assertEqual(steps[build]['with']['labels'],
                                 f'org.gelabber.source-tree=${{{{ steps.trees.outputs.{name} }}}}')
                self.assertIn(f'minio_source.py source {name})', steps['Sources']['run'])


def run_step(workflow, job, name, python):
    """A step's own run: text under bash -e, as GitHub runs it without a
    shell:, with `python` as the body of a fake python3: (exit code, outputs)."""
    script = next(step for step in load(workflow)['jobs'][job]['steps'] if step.get('name') == name)['run']
    with tempfile.TemporaryDirectory() as tmp:
        fake = Path(tmp) / 'python3'
        fake.write_text('#!/bin/sh\n' + python)
        fake.chmod(0o755)
        output = Path(tmp) / 'output'
        result = subprocess.run(['bash', '--noprofile', '--norc', '-e', '-c', script], capture_output=True,
                                env={**os.environ, 'GITHUB_OUTPUT': str(output),
                                     'PATH': f'{tmp}{os.pathsep}{os.environ["PATH"]}'})
        return result.returncode, output.read_text() if output.exists() else ''


class StepsFail(unittest.TestCase):
    """A failing minio_source.py fails its step, not an empty output (review of #1116)."""

    def test_a_failing_plan_fails_the_plan_step(self):
        self.assertNotEqual(run_step('publish-minio.yml', 'plan', 'Plan', 'exit 3\n')[0], 0)
        self.assertEqual(run_step('publish-minio.yml', 'plan', 'Plan', 'echo minio=keep\necho build=false\n'),
                         (0, 'minio=keep\nbuild=false\n'))

    def test_failing_sources_fail_the_step_instead_of_an_empty_label(self):
        self.assertNotEqual(run_step('minio-images.yml', 'build', 'Sources', 'exit 3\n')[0], 0)
        self.assertEqual(run_step('minio-images.yml', 'build', 'Sources', 'echo "$3"\n'),
                         (0, 'minio=minio\nmc=mc\n'))


if __name__ == '__main__':
    unittest.main()
