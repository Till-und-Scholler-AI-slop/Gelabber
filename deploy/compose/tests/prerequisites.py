"""What a test does when the machine lacks a program or service it needs
(GEL-375): on a developer's machine it is skipped and says why; in CI it
fails, so a missing package cannot turn a run green. CI means the variable
`CI` is set to anything but '', '0' or 'false' (GitHub Actions sets 'true').

    @requires(shutil.which('age'), 'age')
    def test_real_age_round_trip(self): ...

    caddy = shutil.which('caddy') or missing('caddy')
"""
import functools
import os
import unittest


def in_ci():
    """`CI` is set, and not to '', '0' or 'false' (in any case)."""
    return os.environ.get('CI', '').strip().lower() not in ('', '0', 'false')


def missing(what, ci=None):
    """Ends the test where `what` is missing: skipped, or failed in CI."""
    if not (in_ci() if ci is None else ci):
        raise unittest.SkipTest(f'{what} is missing on this machine (with CI set, this fails)')
    raise AssertionError(f'{what} is missing; CI must provide it (GEL-375). '
                         '(CI is set; unset it to skip here)')


def requires(present, what, ci=None):
    """unittest.skipUnless(present, ...) for a test or a class of tests,
    except that in CI each test fails."""
    if present:
        return lambda test: test

    def decorate(test):
        if isinstance(test, type):
            for name in [name for name in vars(test) if name.startswith('test')]:
                setattr(test, name, decorate(getattr(test, name)))
            return test

        @functools.wraps(test)
        def without(self, *args, **kwargs):
            missing(what, ci)
        return without
    return decorate
