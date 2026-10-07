#!/usr/bin/env bash
# Cargo test runner for CI diagnostics: runs the test binary under gdb and
# prints every thread's stack when it stops on a signal.
exec gdb -q -batch -ex run -ex "thread apply all bt 40" --args "$@"
