"""Which jobs and steps of desktop-native.yml run in each of its modes.

The callers (ci.yml's `desktop` job, desktop-release.yml's `build` job) set
its inputs from the event; desktop-native.yml's `if:` conditions read them.
This evaluates both from the parsed workflows for a run where every step
that runs succeeds, and holds the result as a table, so an edit to either
side cannot change what a mode packages, signs or checks without changing
the table here too.
"""
import unittest

from workflow_data import load, names
from workflow_expressions import condition, evaluate

WORKFLOW = 'desktop-native.yml'

# mode: (calling workflow, its job, the event's context, outputs of the caller's other jobs)
PULL_REQUEST = {'github.event_name': 'pull_request', 'github.ref': 'refs/pull/1/merge'}
DISPATCH_TAG = {'github.event_name': 'workflow_dispatch', 'github.ref': 'refs/tags/v0.14.1',
                'github.event.release.tag_name': '', 'inputs.release_tag': 'v0.14.1'}
MODES = {
    'pull request': ('ci.yml', 'desktop', PULL_REQUEST,
                     {'needs.changes.outputs.candidate': 'false'}),
    'release candidate': ('ci.yml', 'desktop', PULL_REQUEST,
                          {'needs.changes.outputs.candidate': 'true'}),
    'merge queue': ('ci.yml', 'desktop', {'github.event_name': 'merge_group',
                                         'github.ref': 'refs/heads/gh-readonly-queue/v0.14/pr-1-0123'},
                    {'needs.changes.outputs.candidate': ''}),
    'push to v*': ('ci.yml', 'desktop', {'github.event_name': 'push', 'github.ref': 'refs/heads/v0.14'},
                   {'needs.changes.outputs.candidate': ''}),
    'push to main': ('ci.yml', 'desktop', {'github.event_name': 'push', 'github.ref': 'refs/heads/main'},
                     {'needs.changes.outputs.candidate': ''}),
    'manual run': ('ci.yml', 'desktop', {'github.event_name': 'workflow_dispatch',
                                        'github.ref': 'refs/heads/ci/gel-395-x'},
                   {'needs.changes.outputs.candidate': ''}),
    # release.yml dispatches desktop-release.yml on the tag.
    'release build': ('desktop-release.yml', 'build', DISPATCH_TAG, {'needs.candidate.outputs.run': ''}),
    'release from prebuilt': ('desktop-release.yml', 'build', DISPATCH_TAG,
                              {'needs.candidate.outputs.run': '46000000000'}),
}
ALL = frozenset(MODES)
CI = frozenset(mode for mode, (caller, *_) in MODES.items() if caller == 'ci.yml')
BUILT = ALL - {'release from prebuilt'}
PACKAGED = ALL - {'pull request', 'merge queue'}

# The table: job -> modes it runs in.
JOBS = {
    'libwebrtc-linux': BUILT,
    'libwebrtc-windows': BUILT,
    'core-linux': BUILT,
    'notices': BUILT,
    'core-windows': BUILT,
    'prebuilt': {'release from prebuilt'},
    # The pacman repository: not for a candidate, which desktop-release.yml repeats.
    'arch-repo': PACKAGED - {'release candidate'},
}
# (job, step) -> modes it runs in, for every step that does not run in all
# of its job's modes.
STEPS = {
    # The libwebrtc-packages release is written after pushes and releases only.
    ('libwebrtc-linux', 'Is the package stored?'): BUILT & PACKAGED - {'release candidate'},
    ('libwebrtc-windows', 'Is the package stored?'): BUILT & PACKAGED - {'release candidate'},
    # A release build runs one loopback call; CI ran the rest on the same tree.
    **{('core-linux', step): CI for step in (
        'Format', 'Clippy', 'Install cargo-nextest', 'Loopback call against mediasoup 0.33',
        'Flaky tests', 'Upload the test report', 'Loopback call with the GStreamer H264 encoder',
        'Loopback call with the GStreamer H264 decoder', 'Loopback call with GStreamer AV1 codecs',
        'Loopback call with failing GStreamer AV1 codecs', 'Screen capture through the ScreenCast portal',
        'Voice through PulseAudio devices')},
    ('core-linux', 'Voice test backtrace'): set(),
    ('core-linux', 'Loopback call of the release build'): {'release build'},
    ('core-linux', 'Package the native app'): PACKAGED - {'release from prebuilt'},
    ('core-linux', 'Upload the native app'): PACKAGED - {'release from prebuilt'},
    ('core-windows', 'Native installer'): PACKAGED - {'release from prebuilt'},
    # A candidate's installer is signed once its tag exists (prebuilt).
    ('core-windows', 'Sign the native update'): PACKAGED - {'release from prebuilt', 'release candidate'},
    ('core-windows', 'Native update signature'): PACKAGED - {'release from prebuilt', 'release candidate'},
    ('core-windows', 'Install, update and remove like a user'): PACKAGED - {'release from prebuilt'},
    ('core-windows', 'Upload the native app'): PACKAGED - {'release from prebuilt'},
    ('core-windows', 'Loopback call with libwebrtc logs'): set(),
    ('core-windows', 'Platform tests of the app'): CI,
    ('core-windows', 'Clippy'): CI,
    ('arch-repo', 'Rebuild the repository for upload'): {'release build', 'release from prebuilt'},
}


def inputs_from(mode):
    """desktop-native.yml's inputs as the mode's caller sets them."""
    caller, job_id, event, outputs = MODES[mode]
    job = load(caller)['jobs'][job_id]
    declared = load(WORKFLOW)['on']['workflow_call']['inputs']
    context = {**event, **outputs}

    def lookup(path):
        if path not in context:
            raise ValueError(f'{caller} {job_id}: no value for {path} in mode {mode!r}')
        return context[path]

    unknown = set(job.get('with', {})) - set(declared)
    if unknown:
        raise AssertionError(f'{caller} {job_id} passes inputs {WORKFLOW} does not have: {unknown}')
    values = {name: spec.get('default', False if spec.get('type') == 'boolean' else '')
              for name, spec in declared.items()}
    for name, value in job.get('with', {}).items():
        if isinstance(value, str) and '${{' in value:
            value = evaluate(value, lookup)
        if declared[name].get('type') == 'boolean' and not isinstance(value, bool):
            raise AssertionError(f'{caller} {job_id}: {name} is {value!r}, not a boolean')
        values[name] = value
    return values, event


def needs_of(job):
    needs = job.get('needs', [])
    return [needs] if isinstance(needs, str) else needs


def runs(mode):
    """{job id: [names of the steps that run]} for the jobs that run in `mode`."""
    workflow = load(WORKFLOW)
    inputs, event = inputs_from(mode)
    results = {}
    jobs = dict(workflow['jobs'])
    ran = {}
    while jobs:
        ready = [job_id for job_id, job in jobs.items() if all(need in results for need in needs_of(job))]
        if not ready:
            raise AssertionError(f'circular needs among {sorted(jobs)}')
        for job_id in ready:
            job = jobs.pop(job_id)
            needs = needs_of(job)

            def job_lookup(path, needs=needs):
                if path.startswith('inputs.'):
                    return inputs[path.removeprefix('inputs.')]
                if path in ('github.event_name', 'github.ref'):
                    return event[path]
                parts = path.split('.')
                if parts[0] == 'needs' and parts[1] in needs and parts[2] == 'result':
                    return results[parts[1]]
                raise ValueError(f'{job_id}: cannot tell {path}')

            status_ok = all(results[need] == 'success' for need in needs)
            if not condition(job.get('if'), job_lookup, status_ok):
                results[job_id] = 'skipped'
                continue
            results[job_id] = 'success'
            outcomes, steps = {}, []
            for step in job.get('steps', []):
                def step_lookup(path):
                    parts = path.split('.')
                    if parts[0] == 'steps':
                        if parts[2] in ('outcome', 'conclusion'):
                            return outcomes.get(parts[1], 'skipped')
                        if parts[2] == 'outputs':
                            return ''
                    return job_lookup(path)

                if condition(step.get('if'), step_lookup):
                    steps.append(step['name'])
                    if 'id' in step:
                        outcomes[step['id']] = 'success'
                elif 'id' in step:
                    outcomes[step['id']] = 'skipped'
            ran[job_id] = steps
    return ran


class Modes(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.runs = {mode: runs(mode) for mode in MODES}

    def test_each_mode_runs_its_jobs(self):
        found = {job_id: {mode for mode in MODES if job_id in self.runs[mode]}
                 for job_id in load(WORKFLOW)['jobs']}
        self.assertEqual(found, {job: set(modes) for job, modes in JOBS.items()})

    def test_each_mode_runs_its_steps(self):
        found = {}
        for job_id, job in load(WORKFLOW)['jobs'].items():
            for step in job.get('steps', []):
                modes = {mode for mode in MODES if step['name'] in self.runs[mode].get(job_id, [])}
                if modes != set(JOBS[job_id]):
                    found[(job_id, step['name'])] = modes
        self.assertEqual(found, {key: set(modes) for key, modes in STEPS.items()})

    def test_step_names_are_unique_in_a_job(self):
        for job_id, job in load(WORKFLOW)['jobs'].items():
            names = [step.get('name') for step in job.get('steps', [])]
            with self.subTest(job=job_id):
                self.assertNotIn(None, names)
                self.assertEqual(len(names), len(set(names)))


class WindowsPackage(unittest.TestCase):
    """What part 1 of the app's THIRD-PARTY-NOTICES.txt says about the job
    that builds the Windows package (desktop/packaging/third-party-notices.py
    reads the build settings and the check script; this holds the wiring)."""
    INSTALLER = 'gelabber-native-windows-x64-setup.exe'
    ARTIFACT = 'gelabber-native-windows-x64'
    CHECK = './desktop/packaging/native/windows/check-binaries.ps1'
    SETTINGS = 'desktop/native/windows-build.env'

    @classmethod
    def setUpClass(cls):
        cls.workflow = load(WORKFLOW)
        cls.windows = cls.workflow['jobs']['core-windows']

    def test_the_job_builds_with_the_build_settings(self):
        steps = self.windows['steps']
        loads = [index for index, step in enumerate(steps)
                 if self.SETTINGS in str(step.get('run', '')) and 'GITHUB_ENV' in str(step.get('run', ''))]
        self.assertEqual(len(loads), 1)
        self.assertNotIn('if', steps[loads[0]])
        builds = [index for index, step in enumerate(steps)
                  if 'cargo ' in str(step.get('run', '')) or 'rust-cache' in step.get('uses', '')]
        self.assertTrue(builds)
        self.assertLess(loads[0], min(builds))
        # Nothing sets the variables again.
        for scope, env in [('workflow', self.workflow.get('env', {})), ('job', self.windows.get('env', {})),
                           *[(step['name'], step.get('env', {})) for step in steps]]:
            with self.subTest(scope=scope):
                self.assertFalse({'RUSTFLAGS', 'MESON_ARGS', 'CARGO_ENCODED_RUSTFLAGS'} & set(env))
        for step in steps[loads[0] + 1:]:
            with self.subTest(step=step['name']):
                self.assertNotRegex(str(step.get('run', '')), r'RUSTFLAGS|MESON_ARGS')

    def test_every_build_runs_the_check_of_the_binaries(self):
        named = [step['name'] for step in self.windows['steps'] if step.get('run') == self.CHECK]
        self.assertEqual(len(named), 1)
        for mode in JOBS['core-windows']:
            with self.subTest(mode=mode):
                self.assertIn(named[0], runs(mode)['core-windows'])

    def test_a_release_takes_the_windows_installer_from_that_job_only(self):
        release = load('desktop-release.yml')['jobs']
        upload = release['release-asset-windows']
        self.assertEqual(needs_of(upload), ['build'])
        self.assertEqual(release['build']['uses'], f'./.github/workflows/{WORKFLOW}')
        # The jobs of the build that upload the installer: core-windows, and
        # prebuilt, which signs core-windows' installer of the candidate run and builds nothing.
        uploaders = {job_id for job_id, job in self.workflow['jobs'].items() for step in job.get('steps', [])
                     if step.get('uses', '').startswith('actions/upload-artifact@')
                     and step['with']['name'] == self.ARTIFACT}
        self.assertEqual(uploaders, {'core-windows', 'prebuilt'})
        prebuilt = ' '.join(str(step.get('run', '')) for step in self.workflow['jobs']['prebuilt']['steps'])
        for mark in ('cargo ', 'makensis', 'build-installer'):
            self.assertNotIn(mark, prebuilt)
        downloads = [step['with'] for step in self.workflow['jobs']['prebuilt']['steps']
                     if step.get('uses', '').startswith('actions/download-artifact@')
                     and step['with']['name'] == self.ARTIFACT]
        self.assertEqual([d['run-id'] for d in downloads], ['${{ inputs.prebuilt_run }}'])
        # No other job of any workflow makes or handles the installer.
        allowed = {(WORKFLOW, 'core-windows'), (WORKFLOW, 'prebuilt'), ('desktop-release.yml', 'release-asset-windows')}
        found = [(name, job_id) for name in names() for job_id, job in load(name).get('jobs', {}).items()
                 if (name, job_id) not in allowed
                 and any(self.INSTALLER in str(step.get('run', '')) + str(step.get('with', ''))
                         or 'build-installer.ps1' in str(step.get('run', '')) for step in job.get('steps', []))]
        self.assertEqual(found, [])


class Expressions(unittest.TestCase):
    """The evaluator against the rules of GitHub's expression docs."""

    def value(self, text, **context):
        return evaluate(text, lambda path: context[path.replace('.', '_').replace('-', '_')])

    def test_operators_return_an_operand(self):
        self.assertEqual(self.value("inputs.tag || 'none'", inputs_tag=''), 'none')
        self.assertEqual(self.value("inputs.tag && 'set'", inputs_tag='v1'), 'set')
        self.assertEqual(self.value("inputs.tag && 'set'", inputs_tag=''), '')

    def test_loose_equality(self):
        self.assertIs(self.value("'Push' == 'push'"), True)
        self.assertIs(self.value("inputs.flag == 'true'", inputs_flag=True), False)
        self.assertIs(self.value('inputs.flag == 1', inputs_flag=True), True)
        self.assertIs(self.value("'' == 0"), True)
        self.assertIs(self.value("needs.x-y.outputs.n != ''", needs_x_y_outputs_n='1'), True)

    def test_precedence_and_negation(self):
        text = "(github.event_name != 'pull_request' && github.event_name != 'merge_group') || inputs.c"
        for event, flag, expected in (('pull_request', False, False), ('pull_request', True, True),
                                      ('merge_group', False, False), ('push', False, True)):
            with self.subTest(event=event, flag=flag):
                self.assertEqual(bool(self.value(text, github_event_name=event, inputs_c=flag)), expected)
        self.assertIs(self.value('!inputs.c && true', inputs_c=False), True)

    def test_status_functions(self):
        lookup = {'steps.a.outcome': 'success'}.__getitem__
        self.assertTrue(condition("steps.a.outcome == 'success'", lookup))
        self.assertFalse(condition("steps.a.outcome == 'success'", lookup, status_ok=False))
        self.assertTrue(condition("${{ !cancelled() && steps.a.outcome == 'success' }}", lookup, status_ok=False))
        self.assertFalse(condition('failure()', lookup))

    def test_what_it_cannot_read_raises(self):
        for text in ("inputs.a ~ 'b'", "fromJSON('[1]')[0]", "'open", 'a == ', 'x ${{ y }}'):
            with self.subTest(text=text):
                with self.assertRaises((ValueError, KeyError)):
                    self.value(text, inputs_a='', a='', y='')


if __name__ == '__main__':
    unittest.main()
