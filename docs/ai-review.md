# AI review

PRs to the version branches (`v*`) need no human approval. Instead,
`.github/workflows/ai-review.yml` reviews each PR with Claude
(`anthropics/claude-code-action`, Claude Opus 5.5) and sets the commit status
`ai-review`:

- `success` when nothing is blocking,
- `failure` when it found blocking defects,
- `error` when no verdict came back.

It writes its findings into the PR's "AI review" comment. PRs to `main`
(releases, hotfixes) still need a human approval.

## When it runs

- **When a review runs:** a review runs once a PR is ready for review, that
  is, when it is opened as ready or a draft is marked ready.
- **After a push:** a push to a ready PR sets the status to `pending` with a
  note and starts no review. For a new review, set the PR to draft and back to
  ready for review.
  - Work in a draft while you still push.
  - Each ready costs about one to two dollars.
- **Retargeting:** retargeting a PR to a version branch reviews it as well.

## What it checks

It reads the diff, the commit messages and the code around them in the PR
head. It checks them against `AGENTS.md` of the PR's base. It blocks only on
defects it can show:

- bugs,
- security problems,
- broken `AGENTS.md` rules,
- claims the PR does not keep.

Style and uncertain points go in as notes. It reviews AI-written code with an
AI, so it catches mistakes, not wrong directions. A human decides those.

## Where the workflow comes from

`pull_request_target` always runs the workflow file as it is on `main`, with
`GITHUB_REF` set to `main`. This holds whatever the PR's base is. So a change
to `ai-review.yml` takes effect only once it is on `main`. The PR's base, not
`main`, is checked out for `AGENTS.md`.

## Setup (once, by an admin)

1. **Environment:** Settings → Environments → new environment `ai-review`.
   Under deployment branches, allow only the selected branch `main`.
2. **Secret:** add `ANTHROPIC_API_KEY` to that environment, not to the
   repository. Use a key of its own, in a workspace with a monthly spend
   limit.
3. **Check before you rely on it:** with `ai-review.yml` on `main`, open a
   test PR to a version branch that also touches `AGENTS.md`. Check two
   things:
   - The status `ai-review` shows up and can be chosen as a required check.
   - GitHub still asks for an owner's approval when approvals are 0.
4. **Rulesets:**
   - **`Version branches`:**
     - Require the status checks `ci-ok` and `ai-review`.
     - Set the approvals to 0 and keep "Require review from Code Owners".
     - Bypass: the role Repository admin, for pull requests only. That lets
       an owner merge past a wrong block; GitHub marks the merge as a bypass.
   - **`Main Protection`:**
     - Require the status check `ci-ok`.
     - Set the Organization admin bypass from "always" to "for pull requests
       only".

## Who can use the key

- **Other workflows:** the environment gives the key only to runs on `main`.
  `main` changes only through reviewed PRs, and `.github/` needs an owner's
  approval (`.github/CODEOWNERS`).
- **Claude:**
  - It gets read tools only: no shell, no network, no skills and no subagents.
    A PR could bring its own skills and subagents, with hooks.
  - Reads of `.git/` are denied, because the action writes the job's token
    there. So are reads of `/proc`, the runner's temp folder and Claude's
    config.
  - The project's settings are not loaded.
  - The token for statuses and comments is passed only to the steps that need
    it.
- **Forks:** a fork's PR gets the status `failure` with a note. A maintainer
  takes its commits into a branch of the repository, and that PR gets
  reviewed.
- **The PR's author:** they can try to talk the model into `pass` through the
  PR's text.
- **People with write access:** they can also set the status themselves. This
  is a hurdle against mistakes, not against a malicious collaborator.
