"""What publish-minio.yml publishes, and what image_set.py promote waits for
(.github/workflows/scripts/minio_source.py, GEL-402).

minio_source.py runs as CI runs it, in a scratch repository with real git and
a fake docker that keeps a registry: it accepts only the calls the script
should make, pushes only what was tagged, and a pushed image carries the
label of the build it was tagged from. A bare repository is its origin, with
main.
"""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import textwrap
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / '.github/workflows/scripts/minio_source.py'
ENV = {**os.environ, 'GIT_CONFIG_GLOBAL': os.devnull, 'GIT_CONFIG_NOSYSTEM': '1', 'GITHUB_REPOSITORY': 'O/R'}
MINIO, MC = 'ghcr.io/o/r/minio:R1', 'ghcr.io/o/r/mc:R2'
FORMAT = '{{.Manifest.Digest}} {{.Manifest.MediaType}} {{json .Image.Config.Labels}}'

FAKE_DOCKER = textwrap.dedent('''\
    #!/usr/bin/env python3
    import json, os, sys
    path = os.environ['FAKE_DOCKER_STATE']
    state = json.load(open(path))
    state['calls'].append(sys.argv[1:])
    def done(code=0):
        json.dump(state, open(path, 'w'))
        sys.exit(code)
    args = sys.argv[1:]
    if args[:3] == ['buildx', 'imagetools', 'inspect'] and len(args) == 6 and args[4:] == ['--format', FORMAT]:
        found = state['registry'].get(args[3])
        if found == 'error':
            print('ERROR: unauthorized', file=sys.stderr); done(1)
        if found is None:
            print(f'ERROR: {args[3]}: not found', file=sys.stderr); done(1)
        labels = {'org.gelabber.source-tree': found['tree']} if found['tree'] else None
        kind = 'application/vnd.oci.image.index.v1+json' if found.get('index') else \
            'application/vnd.docker.distribution.manifest.v2+json'
        print(found['digest'], kind, json.dumps(None if found.get('index') else labels)); done()
    if args[0] == 'tag' and len(args) == 3 and args[1] in state['local']:
        state['tagged'][args[2]] = args[1]; done()
    if args[0] == 'push' and len(args) == 2 and args[1] in state['tagged']:
        if state['registry'].get(args[1]) == 'error' or args[1] in state.get('push_fails', []):
            print('denied', file=sys.stderr); done(1)
        state['pushes'] = state.get('pushes', 0) + 1
        state['registry'][args[1]] = {'digest': 'sha256:' + str(state['pushes']) * 64,
                                      'tree': state['local'][state['tagged'][args[1]]]}
        done()
    print('unexpected docker call:', args, file=sys.stderr)
    done(99)
''').replace('FORMAT', repr(FORMAT))


class Registry:
    """A scratch repository with both image folders, compose.yaml, and the
    fake docker's state."""

    def __init__(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name) / 'repo'
        for folder, text in (('deploy/compose/minio', 'FROM golang AS minio\n'),
                             ('deploy/compose/mc', 'FROM golang AS mc\n')):
            (self.root / folder).mkdir(parents=True)
            (self.root / folder / 'Dockerfile').write_text(text)
        (self.root / '.github/workflows').mkdir(parents=True)
        (self.root / '.github/workflows/minio-images.yml').write_text('name: build\n')
        (self.root / 'deploy/compose/compose.yaml').write_text(
            'services:\n'
            '  minio:\n    image: ${GELABBER_MINIO_IMAGE:-' + MINIO + '}\n'
            '  minio-init:\n    image: ${GELABBER_MC_IMAGE:-' + MC + '}\n')
        origin = Path(self.tmp.name) / 'origin.git'
        subprocess.run(['git', 'init', '-q', '--bare', str(origin)], env=ENV, check=True)
        self.git('init', '-q', '-b', 'main')
        self.git('remote', 'add', 'origin', str(origin))
        self.commit()
        bin_dir = Path(self.tmp.name) / 'bin'
        bin_dir.mkdir()
        (bin_dir / 'docker').write_text(FAKE_DOCKER)
        (bin_dir / 'docker').chmod(0o755)
        self.state_path = Path(self.tmp.name) / 'docker.json'
        self.env = {**ENV, 'FAKE_DOCKER_STATE': str(self.state_path),
                    'PATH': f'{bin_dir}{os.pathsep}{os.environ["PATH"]}'}
        self.save({'calls': [], 'registry': {}, 'tagged': {},
                   'local': {'gelabber/minio:R1': self.tree('minio'), 'gelabber/mc:R2': self.tree('mc')}})

    def git(self, *args):
        return subprocess.run(['git', '-c', 'user.name=t', '-c', 'user.email=t@t', *args], cwd=self.root,
                              env=ENV, check=True, capture_output=True, text=True).stdout.strip()

    def commit(self, push=True):
        """A commit on main, pushed to origin unless `push` is false."""
        self.git('add', '-A')
        self.git('commit', '-q', '--allow-empty', '-m', 'x')
        if push:
            self.git('push', '-q', 'origin', 'HEAD:main')
        return self.git('rev-parse', 'HEAD')

    def tree(self, name, rev='HEAD'):
        """What the label holds: the folder's tree and the build workflow's blob."""
        return '+'.join(self.git('rev-parse', f'{rev}:deploy/compose/{name}',
                                 f'{rev}:.github/workflows/minio-images.yml').split())

    def save(self, state):
        self.state_path.write_text(json.dumps(state))

    def state(self):
        return json.loads(self.state_path.read_text())

    def publish_registry(self, **images):
        state = self.state()
        state['registry'] = images
        self.save(state)

    def run(self, *args):
        result = subprocess.run([sys.executable, str(SCRIPT), *args], cwd=self.root, env=self.env,
                                capture_output=True, text=True)
        return result.returncode, result.stdout, result.stderr

    def plan(self):
        code, out, err = self.run('plan')
        if code:
            return code, err
        return code, dict(line.split('=', 1) for line in out.splitlines())


def built(tree, digest='1'):
    return {'digest': 'sha256:' + digest * 64, 'tree': tree}


class Plan(unittest.TestCase):
    def setUp(self):
        self.repo = Registry()
        self.addCleanup(self.repo.tmp.cleanup)

    def test_a_missing_tag_is_pushed(self):
        code, plan = self.repo.plan()
        self.assertEqual((code, plan['minio'], plan['mc'], plan['build']), (0, 'push', 'push', 'true'))

    def test_a_tag_built_from_this_commit_is_kept(self):
        self.repo.publish_registry(**{MINIO: built(self.repo.tree('minio')), MC: built(self.repo.tree('mc'))})
        code, plan = self.repo.plan()
        self.assertEqual((code, plan['minio'], plan['mc'], plan['build']), (0, 'keep', 'keep', 'false'))

    def test_a_changed_folder_or_a_missing_label_is_replaced(self):
        old_minio = self.repo.tree('minio')
        (self.repo.root / 'deploy/compose/minio/Dockerfile').write_text('FROM golang:newer AS minio\n')
        self.repo.commit()
        self.repo.publish_registry(**{MINIO: built(old_minio), MC: built('')})
        code, plan = self.repo.plan()
        self.assertEqual((code, plan['minio'], plan['mc'], plan['build']), (0, 'replace', 'replace', 'true'))

    def test_a_changed_build_workflow_rebuilds_both(self):
        self.repo.publish_registry(**{MINIO: built(self.repo.tree('minio')), MC: built(self.repo.tree('mc'))})
        (self.repo.root / '.github/workflows/minio-images.yml').write_text('name: build\n# platforms\n')
        self.repo.commit()
        code, plan = self.repo.plan()
        self.assertEqual((code, plan['minio'], plan['mc']), (0, 'replace', 'replace'))

    def test_an_older_commit_publishes_nothing(self):
        # A re-run of an older publish-minio run must not roll the tag back.
        older = self.repo.commit()
        (self.repo.root / 'deploy/compose/minio/Dockerfile').write_text('FROM golang:newer AS minio\n')
        self.repo.commit()
        self.repo.publish_registry(**{MINIO: built(self.repo.tree('minio')), MC: built(self.repo.tree('mc'))})
        self.repo.git('checkout', '-q', older)
        code, err = self.repo.plan()
        self.assertNotEqual(code, 0)
        self.assertIn('main has moved on', err)
        code, _, err = self.repo.run('publish', 'replace', 'keep')
        self.assertNotEqual(code, 0)
        self.assertIn('main has moved on', err)
        self.assertEqual([call for call in self.repo.state()['calls'] if call[0] in ('tag', 'push')], [])

    def test_a_multi_platform_index_is_an_error_not_a_missing_label(self):
        self.repo.publish_registry(**{MINIO: dict(built(self.repo.tree('minio')), index=True),
                                      MC: built(self.repo.tree('mc'))})
        code, err = self.repo.plan()
        self.assertNotEqual(code, 0)
        self.assertIn('multi-platform index', err)

    def test_a_registry_error_on_either_image_stops_the_plan(self):
        for broken in (MINIO, MC):
            with self.subTest(broken=broken):
                self.repo.publish_registry(**{MINIO: built(self.repo.tree('minio')),
                                              MC: built(self.repo.tree('mc')), broken: 'error'})
                code, err = self.repo.plan()
                self.assertNotEqual(code, 0)
                self.assertIn('Registry inspection failed', err)


class Publish(unittest.TestCase):
    def setUp(self):
        self.repo = Registry()
        self.addCleanup(self.repo.tmp.cleanup)

    def calls(self, verb):
        return [call for call in self.repo.state()['calls'] if call[0] == verb]

    def test_push_and_replace_tag_the_image_built_for_that_tag(self):
        self.repo.publish_registry(**{MC: built('old-mc-tree')})
        code, _, err = self.repo.run('publish', 'push', 'replace')
        self.assertEqual(code, 0, err)
        self.assertEqual(self.calls('tag'), [['tag', 'gelabber/minio:R1', MINIO], ['tag', 'gelabber/mc:R2', MC]])
        self.assertEqual(self.calls('push'), [['push', MINIO], ['push', MC]])
        registry = self.repo.state()['registry']
        self.assertEqual((registry[MINIO]['tree'], registry[MC]['tree']), (self.repo.tree('minio'), self.repo.tree('mc')))

    def test_a_build_from_other_sources_fails_after_the_push(self):
        # A stale artifact or a wrong labels: input: the tag must not stay quiet.
        state = self.repo.state()
        state['local']['gelabber/minio:R1'] = 'stale+sources'
        self.repo.save(state)
        code, _, err = self.repo.run('publish', 'push', 'keep')
        self.assertNotEqual(code, 0)
        self.assertIn("does not carry this commit's sources", err)

    def test_keep_pushes_nothing(self):
        code, _, _ = self.repo.run('publish', 'keep', 'keep')
        self.assertEqual((code, self.calls('tag'), self.calls('push')), (0, [], []))

    def test_a_failure_on_the_second_image_is_finished_by_the_next_run(self):
        self.repo.publish_registry(**{MINIO: built('old'), MC: built('old')})
        state = self.repo.state()
        state['push_fails'] = [MC]
        self.repo.save(state)
        code, _, _ = self.repo.run('publish', 'replace', 'replace')
        self.assertNotEqual(code, 0)
        state = self.repo.state()
        state['push_fails'] = []
        self.repo.save(state)
        self.assertEqual(self.repo.plan()[1]['minio'], 'keep')
        self.assertEqual(self.repo.plan()[1]['mc'], 'replace')

    def test_an_unknown_action_pushes_nothing(self):
        code, _, _ = self.repo.run('publish', 'push', 'later')
        self.assertNotEqual(code, 0)
        self.assertEqual(self.calls('push'), [])


def load():
    spec = importlib.util.spec_from_file_location('minio_source', SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class WaitFor(unittest.TestCase):
    """promote waits, with a bound, for the tag to be built from the release commit."""

    def wait(self, answers, bound=60):
        source = load()
        now, slept = [0], []

        def sleep(seconds):
            slept.append(seconds)
            now[0] += seconds
        with patch.object(source, 'published', side_effect=answers):
            result = source.wait_for(MINIO, 'tree', bound=bound, every=30, clock=lambda: now[0], sleep=sleep)
        return result, slept

    def test_the_digest_once_the_tag_is_built_from_the_tree(self):
        self.assertEqual(self.wait([('sha256:' + '1' * 64, 'old'), None, ('sha256:' + '2' * 64, 'tree')]),
                         ('sha256:' + '2' * 64, [30, 30]))

    def test_at_once_when_it_already_is(self):
        self.assertEqual(self.wait([('sha256:' + '2' * 64, 'tree')]), ('sha256:' + '2' * 64, []))

    def test_it_gives_up_after_the_bound_naming_the_workflow(self):
        with self.assertRaisesRegex(ValueError, 'run publish-minio.yml on main'):
            self.wait([('sha256:' + '1' * 64, 'old')] * 10, bound=60)

    def test_a_registry_error_is_retried_within_the_bound(self):
        self.assertEqual(self.wait([RuntimeError('unauthorized'), ('sha256:' + '2' * 64, 'tree')]),
                         ('sha256:' + '2' * 64, [30]))


if __name__ == '__main__':
    unittest.main()
