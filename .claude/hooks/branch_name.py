#!/usr/bin/env python3
"""PreToolUse hook: blocks a `git push` of a branch whose name breaks the rule
in AGENTS.md ("Branches and pull requests"). The rule is about branches on
origin, so local branches (e.g. one fetched to review a PR) stay free.

Reads the hook input from stdin; exit code 2 blocks the command and shows
stderr to Claude. Anything it cannot parse is let through.
"""
import json
import re
import shlex
import subprocess
import sys

NAME = re.compile(r"""^(
    (feature|fix|docs|ci|refactor|chore)/(gel-)?[0-9]+-[a-z0-9]+(-[a-z0-9]+)*
  | release/v[0-9]+\.[0-9]+\.[0-9]+
  | hotfix/v[0-9]+\.[0-9]+\.[0-9]+-[a-z0-9]+(-[a-z0-9]+)*
  | sync/main-into-v[0-9]+\.[0-9]+
  | v[0-9]+\.[0-9]+(\.[0-9]+)?
  | main
  | dependabot/.+
)$""", re.X)

RULE = (
    "Branch names follow AGENTS.md (\"Branches and pull requests\"): "
    "<type>/<issue>-<short-description>, lowercase with hyphens, <issue> the Linear ID, "
    "type one of feature, fix, docs, ci, refactor, chore (e.g. fix/gel-21-edit-under-block). "
    "Release work uses release/vX.Y.Z, hotfix/vX.Y.Z-<short-description> and "
    "sync/main-into-vX.Y. Without an issue, open one in Linear first. There is no exception "
    "for names a tool suggests. Rename the branch with `git branch -m <new-name>` and push that."
)

HEREDOC = re.compile(r"<<-?\s*(['\"]?)(\w+)\1[^\n]*\n.*?\n\s*\2\s*(?=\n|$)", re.S)
SEPARATORS = {';', '&&', '||', '|', '&', '(', ')'}
# git options before the subcommand that take a value.
GIT_VALUE_OPTIONS = {'-C', '-c', '--git-dir', '--work-tree', '--namespace'}
PUSH_VALUE_OPTIONS = {'-o', '--push-option', '--repo', '--receive-pack', '--exec'}
PUSH_SKIP = {'--delete', '-d', '--tags', '--all', '--mirror', '--prune'}


def commands(text):
    """Each simple command of a shell line, as a token list."""
    text = HEREDOC.sub('', text).replace('\n', ' ; ')
    lexer = shlex.shlex(text, posix=True, punctuation_chars=True)
    lexer.whitespace_split = True
    try:
        tokens = list(lexer)
    except ValueError:
        return []
    current, result = [], []
    for token in tokens:
        if token in SEPARATORS or set(token) <= set(';&|()'):
            if current:
                result.append(current)
            current = []
        else:
            current.append(token)
    if current:
        result.append(current)
    return result


def git_call(tokens):
    """(options before the subcommand, subcommand, its arguments) or None."""
    for index, token in enumerate(tokens):
        if token == 'git' or token.endswith('/git'):
            break
    else:
        return None
    rest, options = tokens[index + 1:], []
    while rest and rest[0].startswith('-'):
        option = rest.pop(0)
        options.append(option)
        if option in GIT_VALUE_OPTIONS and rest:
            options.append(rest.pop(0))
    if not rest:
        return None
    return options, rest[0], rest[1:]


def git(options, *args):
    try:
        return subprocess.run(['git', *options, *args], capture_output=True, text=True,
                              timeout=10).stdout.strip()
    except (OSError, subprocess.SubprocessError):
        return ''


def positionals(args, value_options=()):
    result, skip = [], False
    for arg in args:
        if skip:
            skip = False
        elif arg == '--':
            continue
        elif arg.startswith('-'):
            skip = arg in value_options
        else:
            result.append(arg)
    return result


def pushed(options, sub, args):
    """The branch names this git call pushes."""
    if sub != 'push' or any(arg in PUSH_SKIP for arg in args):
        return []
    refs = positionals(args, PUSH_VALUE_OPTIONS)[1:]
    if not refs:
        refs = ['HEAD']
    names = []
    for ref in refs:
        ref = ref.lstrip('+')
        source, colon, target = ref.partition(':')
        if colon and not source:
            continue  # `:<branch>` deletes it
        target = target if colon else source
        if not target or target.startswith('refs/tags/'):
            continue
        if target == 'HEAD':
            target = git(options, 'rev-parse', '--abbrev-ref', 'HEAD')
            if not target or target == 'HEAD':
                continue
        target = target.removeprefix('refs/heads/')
        if (git(options, 'rev-parse', '--verify', '--quiet', 'refs/tags/' + target)
                and not git(options, 'rev-parse', '--verify', '--quiet', 'refs/heads/' + target)):
            continue
        names.append(target)
    return names


def violations(command):
    bad = []
    for tokens in commands(command):
        call = git_call(tokens)
        if call:
            bad += [name for name in pushed(*call) if not NAME.match(name)]
    return bad


def main():
    try:
        data = json.load(sys.stdin)
    except ValueError:
        return 0
    command = (data.get('tool_input') or {}).get('command') or ''
    bad = violations(command)
    if not bad:
        return 0
    print(f"Blocked: branch name {', '.join(repr(name) for name in bad)}. {RULE}", file=sys.stderr)
    return 2


if __name__ == '__main__':
    sys.exit(main())
