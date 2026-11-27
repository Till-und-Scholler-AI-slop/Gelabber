"""What a test does when the machine lacks a program or service it needs
(GEL-375): on a developer's machine it is skipped and says why; in CI (`CI`
set, as GitHub Actions sets it) it fails, so a missing package cannot turn a
run green.

    @requires(shutil.which('age'), 'age')
    def test_real_age_round_trip(self): ...

    caddy = shutil.which('caddy') or missing('caddy')
"""
import functools
import os
import unittest


def in_ci():
    """`CI` is set to anything but the empty string."""
    return bool(os.environ.get('CI'))


def missing(what, ci=None):
    """Ends the test where `what` is missing: skipped, or failed in CI."""
    if not (in_ci() if ci is None else ci):
        raise unittest.SkipTest(f'{what} is missing on this machine (with CI set, this fails)')
    raise AssertionError(f'{what} is missing; CI must provide it (GEL-375)')


def requires(present, what, ci=None):
    """unittest.skipUnless(present, ...), except that in CI the test fails."""
    if present:
        return lambda test: test

    def decorate(test):
        @functools.wraps(test)
        def without(self, *args, **kwargs):
            missing(what, ci)
        return without
    return decorate
