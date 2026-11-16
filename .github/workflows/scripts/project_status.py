"""Keeps the Status of issues on the organization project in step with the work.

project-status.yml runs this on issue and pull request events and on manual
dispatch. `decide` turns an event into steps (pure, tested in
deploy/compose/tests/test_project_status.py); `main` carries them out.

    issue assigned                  -> In progress   (from Backlog/Planned/none)
    last assignee removed           -> Planned with a milestone, else Backlog
                                       (from In progress)
    draft PR says Closes #n         -> In progress   (from Backlog/Planned/none)
    PR ready for review, Closes #n  -> In review     (from Backlog/Planned/In progress/none)
    PR back to draft                -> In progress   (from In review)
    PR closed without merge         -> In progress while assigned, else as above
                                       (from In review/In progress)
    PR merged                       -> Done; into a version branch the issue is
                                       closed too, since GitHub only closes
                                       issues for merges into the default branch
    manual dispatch                 -> the given Status, from any

Moves need PROJECT_TOKEN (GITHUB_TOKEN cannot reach projects); closing issues
needs only GITHUB_TOKEN. Without PROJECT_TOKEN, event runs warn and skip the
moves, a manual dispatch fails.
"""
import json
import os
import re
import sys
import urllib.error
import urllib.request

STATUSES = ('Backlog', 'Planned', 'In progress', 'In review', 'Done')
NOT_STARTED = (None, 'Backlog', 'Planned')
# Resolved per issue when applied: In progress while someone is assigned,
# else Planned with a milestone, else Backlog.
RESTING = 'resting'
# GitHub's closing keywords, e.g. "Closes #12", "fixes: #3".
CLOSING = re.compile(r'\b(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?)\s*:?\s+#(\d+)\b', re.IGNORECASE)


def closing_refs(body):
    """Issue numbers a pull request body closes, in order, without repeats."""
    seen = []
    for match in CLOSING.finditer(body or ''):
        number = int(match.group(1))
        if number not in seen:
            seen.append(number)
    return seen


def move(issue, status, only_from=None):
    """Set the issue's Status, only when its current one is in only_from (None: always)."""
    return {'action': 'move', 'issue': issue, 'status': status, 'only_from': only_from}


def close(issue):
    return {'action': 'close', 'issue': issue}


def decide(event_name, event, default_branch):
    """The steps for one event, in order."""
    action = event.get('action')
    if event_name == 'workflow_dispatch':
        inputs = event.get('inputs') or {}
        status = inputs.get('status')
        if status not in STATUSES:
            raise ValueError(f'unknown status {status!r}')
        return [move(int(inputs['issue']), status)]
    if event_name == 'issues':
        issue = event['issue']
        if action == 'assigned':
            return [move(issue['number'], 'In progress', NOT_STARTED)]
        if action == 'unassigned' and not issue.get('assignees'):
            return [move(issue['number'], RESTING, ('In progress',))]
        return []
    if event_name not in ('pull_request', 'pull_request_target'):
        return []
    pull = event['pull_request']
    refs = closing_refs(pull.get('body'))
    if action in ('opened', 'reopened', 'ready_for_review', 'edited'):
        if pull.get('draft'):
            return [move(n, 'In progress', NOT_STARTED) for n in refs]
        return [move(n, 'In review', NOT_STARTED + ('In progress',)) for n in refs]
    if action == 'converted_to_draft':
        return [move(n, 'In progress', ('In review',)) for n in refs]
    if action == 'closed' and pull.get('merged'):
        into_version_branch = pull['base']['ref'] != default_branch
        steps = []
        for n in refs:
            if into_version_branch:
                steps.append(close(n))
            steps.append(move(n, 'Done'))
        return steps
    if action == 'closed':
        return [move(n, RESTING, ('In review', 'In progress')) for n in refs]
    return []


def resting_status(issue):
    if issue.get('assignees'):
        return 'In progress'
    return 'Planned' if issue.get('milestone') else 'Backlog'


class GitHub:
    def __init__(self, token):
        self.token = token
        self.api = os.environ.get('GITHUB_API_URL', 'https://api.github.com')
        self.graphql_url = os.environ.get('GITHUB_GRAPHQL_URL', self.api + '/graphql')

    def request(self, method, url, body=None):
        data = None if body is None else json.dumps(body).encode()
        request = urllib.request.Request(url, data=data, method=method, headers={
            'Authorization': f'Bearer {self.token}',
            'Accept': 'application/vnd.github+json',
            'User-Agent': 'gelabber-project-status',
            'X-GitHub-Api-Version': '2022-11-28',
        })
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                return json.load(response)
        except urllib.error.HTTPError as error:
            raise RuntimeError(f'{method} {url}: {error.code} {error.read().decode(errors="replace")}') from None

    def rest(self, method, path, body=None):
        return self.request(method, self.api + path, body)

    def graphql(self, query, **variables):
        result = self.request('POST', self.graphql_url, {'query': query, 'variables': variables})
        if result.get('errors'):
            raise RuntimeError(f'GraphQL: {result["errors"]}')
        return result['data']


class Project:
    """The Status field of one organization project."""

    def __init__(self, github, owner, number):
        self.github = github
        data = github.graphql('''
            query($owner: String!, $number: Int!) {
              organization(login: $owner) {
                projectV2(number: $number) {
                  id
                  field(name: "Status") {
                    ... on ProjectV2SingleSelectField { id options { id name } }
                  }
                }
              }
            }''', owner=owner, number=number)
        project = data['organization']['projectV2']
        self.id = project['id']
        self.field_id = project['field']['id']
        self.options = {option['name']: option['id'] for option in project['field']['options']}

    def item(self, issue_node_id):
        """The issue's item on this project and its Status; adds the issue if missing."""
        data = self.github.graphql('''
            query($id: ID!) {
              node(id: $id) {
                ... on Issue {
                  projectItems(first: 50) {
                    nodes {
                      id
                      project { id }
                      fieldValueByName(name: "Status") {
                        ... on ProjectV2ItemFieldSingleSelectValue { name }
                      }
                    }
                  }
                }
              }
            }''', id=issue_node_id)
        for node in data['node']['projectItems']['nodes']:
            if node['project']['id'] == self.id:
                return node['id'], (node.get('fieldValueByName') or {}).get('name')
        added = self.github.graphql('''
            mutation($project: ID!, $content: ID!) {
              addProjectV2ItemById(input: {projectId: $project, contentId: $content}) { item { id } }
            }''', project=self.id, content=issue_node_id)
        return added['addProjectV2ItemById']['item']['id'], None

    def set_status(self, item_id, status):
        if status not in self.options:
            raise RuntimeError(f'the project has no Status option {status!r}')
        self.github.graphql('''
            mutation($project: ID!, $item: ID!, $field: ID!, $option: String!) {
              updateProjectV2ItemFieldValue(input: {
                projectId: $project, itemId: $item, fieldId: $field,
                value: {singleSelectOptionId: $option}
              }) { projectV2Item { id } }
            }''', project=self.id, item=item_id, field=self.field_id, option=self.options[status])


def main():
    event_name = os.environ['GITHUB_EVENT_NAME']
    with open(os.environ['GITHUB_EVENT_PATH'], encoding='utf-8') as file:
        event = json.load(file)
    if event_name == 'workflow_dispatch':
        event['inputs'] = {'issue': os.environ['INPUT_ISSUE'], 'status': os.environ['INPUT_STATUS']}
    steps = decide(event_name, event, os.environ['DEFAULT_BRANCH'])
    if not steps:
        print('Nothing to do for this event.')
        return 0
    repo = os.environ['GITHUB_REPOSITORY']
    github = GitHub(os.environ['GITHUB_TOKEN'])
    project_token = os.environ.get('PROJECT_TOKEN', '')
    project = None
    if project_token:
        project = Project(GitHub(project_token), os.environ['PROJECT_OWNER'], int(os.environ['PROJECT_NUMBER']))
    elif event_name == 'workflow_dispatch':
        print('::error::The PROJECT_TOKEN secret is not set, so no card can be moved.')
        return 1
    else:
        print('::warning::The PROJECT_TOKEN secret is not set; cards are not moved.')

    for step in steps:
        issue = github.rest('GET', f'/repos/{repo}/issues/{step["issue"]}')
        label = f'#{step["issue"]}'
        if 'pull_request' in issue:
            print(f'{label} is a pull request, skipped.')
            continue
        if step['action'] == 'close':
            if issue['state'] == 'open':
                github.rest('PATCH', f'/repos/{repo}/issues/{step["issue"]}',
                            {'state': 'closed', 'state_reason': 'completed'})
                print(f'{label} closed as completed.')
            continue
        if project is None:
            continue
        status = resting_status(issue) if step['status'] == RESTING else step['status']
        if issue['state'] == 'closed' and status != 'Done' and event_name != 'workflow_dispatch':
            print(f'{label} is closed, Status left alone.')
            continue
        item_id, current = project.item(issue['node_id'])
        if current == status:
            print(f'{label} is already {status}.')
        elif step['only_from'] is not None and current not in step['only_from']:
            print(f'{label} stays {current} (moves to {status} only from {step["only_from"]}).')
        else:
            project.set_status(item_id, status)
            print(f'{label}: {current or "no Status"} -> {status}')
    return 0


if __name__ == '__main__':
    sys.exit(main())
