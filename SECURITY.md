# Security policy

## Supported versions

Security fixes go into the latest release. Older releases get no fixes, so
please update before you report: the
[release notes](https://github.com/Till-und-Scholler-AI-slop/Gelabber/releases)
say when a release closes a security hole.

## Reporting a vulnerability

Please do not open a public issue, and do not describe the problem in a
pull request.

Report it privately through GitHub: on the repository's
[Security tab](https://github.com/Till-und-Scholler-AI-slop/Gelabber/security),
choose "Report a vulnerability". Only the maintainers can read the report.

It helps to include:

- the affected part (API, gateway, media service, web client, desktop app,
  deployment) and the version,
- what an attacker needs (an account, membership of a server, network
  access) and what they gain,
- steps or a proof of concept.

Two people maintain Gelabber. We read reports as soon as we can, tell you
whether we can reproduce the problem, and name you in the release notes if
you want that.

## Scope

In scope: this repository's code and its default deployment
(`deploy/compose`).

Out of scope: an instance someone else runs (contact its operator),
problems that need an operator's own misconfiguration, and third-party
components without a Gelabber-specific impact (report those upstream).

## For operators

`deploy/README.md` has a "Security" section on what the default deployment
exposes. Release notes list changes an operator must act on under "Upgrade
notes".
