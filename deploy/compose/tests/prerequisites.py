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


def skipped(what):
    return f'{what} is missing on this machine (with CI set, this fails)'


def missing(what, ci=None):
    """Ends the test where `what` is missing: skipped, or failed in CI."""
    if not (in_ci() if ci is None else ci):
        raise unittest.SkipTest(skipped(what))
    raise AssertionError(f'{what} is missing; CI must provide it (GEL-375). '
                         '(CI is set; unset it to skip here)')


def requires(present, what, ci=None):
    """unittest.skipUnless(present, ...) for a test, a class of tests (its
    inherited tests too) or a setUpClass, except that in CI they fail."""
    if present:
        return lambda test: test
    ci = in_ci() if ci is None else ci

    def without(function):
        @functools.wraps(function)
        def missing_here(*args, **kwargs):
            missing(what, ci)
        return missing_here

    def decorate(test):
        if isinstance(test, classmethod):
            return classmethod(without(test.__func__))
        if not isinstance(test, type):
            return without(test)
        if not ci:
            return unittest.skip(skipped(what))(test)
        # Neither the class's own setUpClass nor any of its tests runs.
        test.setUpClass = classmethod(without(lambda cls: None))
        prefix = unittest.TestLoader.testMethodPrefix
        for name in dir(test):
            if name.startswith(prefix) and callable(getattr(test, name)):
                setattr(test, name, without(getattr(test, name)))
        return test
    return decorate
