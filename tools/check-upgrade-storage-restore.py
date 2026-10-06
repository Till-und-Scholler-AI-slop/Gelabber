#!/usr/bin/env python3
"""Local API upgrade, paired PostgreSQL/MinIO restore and old-binary rollback.

Creates only labelled, nonce-owned Docker resources on a local Unix daemon.
Builds frozen Git revisions with their locked Rust toolchain. No production
configuration, credentials, databases, volumes or external endpoints are used.
Slow acceptance drill; intentionally not part of GitHub Actions.
"""
import argparse
import base64
from datetime import datetime, timezone
import hashlib
import http.cookies
import io
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tarfile
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid


POSTGRES = 'postgres:18.6-alpine3.24'
REDIS = 'redis:8.10.1-alpine'
MINIO = 'ghcr.io/till-und-scholler-ai-slop/gelabber/minio:RELEASE.2025-10-15T17-29-55Z'
LABEL = 'org.gelabber.local-restore-drill'
ROOT = Path(__file__).resolve().parents[1]


def sha(data):
    return hashlib.sha256(data).hexdigest()


def equal(actual, expected, label):
    if actual != expected:
        raise AssertionError(f'{label}: {actual!r} != {expected!r}')


def expected_failure(action, prefix):
    """A negative control must fail for its intended cause, not any error."""
    try:
        action()
    except AssertionError as error:
        if not str(error).startswith(prefix):
            raise
        return str(error)
    raise AssertionError(f'negative control unexpectedly accepted: {prefix}')


def loopback_url(url, expected_port):
    parsed = urllib.parse.urlsplit(url)
    if (parsed.scheme != 'http' or parsed.hostname != '127.0.0.1'
            or parsed.port != expected_port or parsed.username or parsed.password):
        raise ValueError('fixture URL must target its owned loopback service')
    return url


def guard_running_binary(binary):
    if not binary.exists() or not Path('/proc').exists():
        return
    identity = binary.stat()
    for process in Path('/proc').iterdir():
        if not process.name.isdigit():
            continue
        try:
            running = (process / 'exe').stat()
            if (identity.st_dev, identity.st_ino) == (running.st_dev, running.st_ino):
                raise ValueError('Cargo output is a running API binary; use another target directory')
        except (FileNotFoundError, PermissionError):
            pass


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


HTTP = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())


def http_request(url, method='GET', data=None, headers=None):
    request = urllib.request.Request(url, data=data, headers=headers or {}, method=method)
    try:
        with HTTP.open(request, timeout=10) as response:
            return response.status, response.headers, response.read()
    except urllib.error.HTTPError as error:
        with error:
            return error.code, error.headers, error.read()


class Client:
    """API cookies are never forwarded to a presigned object-store URL."""
    def __init__(self, port):
        self.port = port
        self.cookies = {}
        self.csrf = None
        self.user = None

    def raw(self, path, method='GET', body=None):
        if not path.startswith('/api/') or path.startswith('//'):
            raise ValueError('an API-relative path is required')
        headers = {'Cookie': '; '.join(f'{key}={value}' for key, value in self.cookies.items())}
        if method not in ('GET', 'HEAD'):
            headers['X-CSRF-Token'] = self.csrf or ''
        if body is not None:
            headers['Content-Type'] = 'application/json'
        status, received, data = http_request(
            f'http://127.0.0.1:{self.port}{path}', method,
            json.dumps(body).encode() if body is not None else None, headers,
        )
        for cookie in received.get_all('Set-Cookie', []):
            parsed = http.cookies.SimpleCookie(cookie)
            self.cookies.update({key: item.value for key, item in parsed.items()})
        return status, received, data

    def api(self, path, method='GET', body=None, status=200):
        actual, _, data = self.raw(path, method, body)
        equal(actual, status, f'{method} {path} status')
        result = json.loads(data) if data else None
        if isinstance(result, dict) and 'csrf_token' in result:
            self.csrf = result['csrf_token']
            self.user = result.get('user')
        return result


class Drill:
    def __init__(self, args):
        self.args = args
        self.output = args.output.resolve()
        # Never overwrite an operator's artifact directory.
        self.output.mkdir(mode=0o700)
        self.nonce = uuid.uuid4().hex
        self.prefix = f'gelabber-restore-{self.nonce}'
        self.containers = []
        self.volumes = []
        self.process = None
        self.api_log = None
        self.secret = uuid.uuid4().hex
        self.bucket = 'drill-attachments'
        self.started = time.monotonic()
        self.report = {
            'schema': 1, 'passed': False, 'nonce': self.nonce,
            'started_at_utc': datetime.now(timezone.utc).isoformat(),
            'runner_sha256': sha(Path(__file__).read_bytes()), 'python_version': sys.version,
            'scope': 'local frozen API binaries, PostgreSQL and paired MinIO volume snapshots; no production rollout, Web/Media image rollback or device acceptance',
            'checks': [], 'binaries': {}, 'images': {}, 'snapshots': {},
        }

    def command(self, args, **kwargs):
        return subprocess.check_output(args, **kwargs)

    def docker(self, *args, **kwargs):
        return self.command(['docker', *args], **kwargs)

    def check(self, label):
        self.report['checks'].append(label)
        print(label, flush=True)

    def preflight(self):
        if os.environ.get('DOCKER_HOST') or os.environ.get('DOCKER_CONTEXT'):
            raise ValueError('Docker endpoint overrides are not supported')
        context = json.loads(self.docker('context', 'inspect'))[0]
        if not context['Endpoints']['docker']['Host'].startswith('unix://'):
            raise ValueError('a local Unix Docker endpoint is required')
        for label, image in [('postgres', POSTGRES), ('redis', REDIS), ('minio', MINIO)]:
            inspected = json.loads(self.docker('image', 'inspect', image))[0]
            self.report['images'][label] = {
                'reference': image, 'id': inspected['Id'], 'repo_digests': inspected.get('RepoDigests', []),
            }
        self.check('local daemon and exact cached pinned service image IDs')

    def build(self, ref, label):
        revision = self.command(['git', 'rev-parse', f'{ref}^{{commit}}'], cwd=ROOT, text=True).strip()
        source = self.output / f'{label}-source'
        source.mkdir()
        archive = self.command(['git', 'archive', revision], cwd=ROOT)
        with tarfile.open(fileobj=io.BytesIO(archive)) as files:
            files.extractall(source, filter='data')
        # Read the frozen revision's toolchain, never adjust pins for the drill.
        import tomllib
        toolchain = tomllib.loads((source / 'rust-toolchain.toml').read_text())['toolchain']['channel']
        target = (self.args.target_dir or self.output / 'target').resolve()
        guard_running_binary(target / 'debug/gelabber-api')
        env = dict(os.environ, CARGO_TARGET_DIR=str(target))
        with (self.output / f'{label}-build.log').open('wb') as log:
            subprocess.run(['cargo', f'+{toolchain}', 'build', '--locked', '-p', 'gelabber-api'],
                           cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
        binary = self.output / f'{label}-api'
        shutil.copy2(target / 'debug/gelabber-api', binary)
        self.report['binaries'][label] = {
            'revision': revision, 'binary_sha256': sha(binary.read_bytes()),
            'git_archive_sha256': sha(archive), 'rust_toolchain': toolchain,
            'rustc_version': self.command(['rustc', f'+{toolchain}', '--version'], text=True).strip(),
            'cargo_version': self.command(['cargo', f'+{toolchain}', '--version'], text=True).strip(),
            'cargo_lock_sha256': sha((source / 'Cargo.lock').read_bytes()),
            'migrations': {p.name: sha(p.read_bytes()) for p in sorted((source / 'api/migrations').glob('*.sql'))},
        }
        self.check(f'{label} frozen source built --locked and executable copied/hash recorded')
        return binary

    def create_container(self, suffix, image, *args, command=()):
        name = f'{self.prefix}-{suffix}'
        cid = self.docker('create', '--name', name, '--label', f'{LABEL}={self.nonce}',
                          *args, image, *command).decode().strip()
        self.containers.append(cid)
        return cid

    def start_store(self, store):
        self.docker('start', store['container'])
        # Docker may allocate a different ephemeral port after stop/start.
        store['port'] = self.port(store['container'], 9000)
        self.wait(lambda: http_request(f'http://127.0.0.1:{store["port"]}/minio/health/ready')[0] == 200,
                  'MinIO ready')

    def volume(self, suffix):
        name = f'{self.prefix}-{suffix}'
        self.docker('volume', 'create', '--label', f'{LABEL}={self.nonce}', name)
        self.volumes.append(name)
        return name

    def port(self, container, port):
        inspected = json.loads(self.docker('inspect', container))[0]
        binding = inspected['NetworkSettings']['Ports'][f'{port}/tcp']
        equal(binding[0]['HostIp'], '127.0.0.1', 'service binds only loopback')
        return int(binding[0]['HostPort'])

    def wait(self, predicate, label, timeout=45):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                if predicate():
                    return
            except (OSError, subprocess.CalledProcessError):
                pass
            time.sleep(0.15)
        raise TimeoutError(label)

    def services(self):
        self.pg = self.create_container('postgres', self.report['images']['postgres']['id'],
            '-e', f'POSTGRES_PASSWORD={self.secret}', '-p', '127.0.0.1::5432',
            '--mount', f'type=volume,src={self.volume("postgres-data")},dst=/var/lib/postgresql')
        self.redis = self.create_container('redis', self.report['images']['redis']['id'],
            '-p', '127.0.0.1::6379')
        for container in (self.pg, self.redis):
            self.docker('start', container)
        self.wait(lambda: self.docker('exec', self.pg, 'pg_isready', '-U', 'postgres'), 'PostgreSQL ready')
        self.pg_port, self.redis_port = self.port(self.pg, 5432), self.port(self.redis, 6379)
        self.report['service_versions'] = {
            'postgres': self.sql('postgres', 'SHOW server_version;'),
            'redis': next(line.split(':', 1)[1] for line in
                          self.docker('exec', self.redis, 'redis-cli', 'INFO', 'server').decode().splitlines()
                          if line.startswith('redis_version:')),
        }
        equal(self.report['service_versions']['postgres'], '18.6', 'pinned PostgreSQL executable version')
        equal(self.report['service_versions']['redis'], '8.10.1', 'pinned Redis executable version')

    def sql(self, database, statement):
        return self.docker('exec', '-i', self.pg, 'psql', '-X', '-q', '-v', 'ON_ERROR_STOP=1',
                           '-U', 'postgres', '-d', database, '-At', input=statement.encode()).decode().strip()

    def database(self, suffix):
        name = f'gb_{suffix}_{self.nonce}'
        self.sql('postgres', f'CREATE DATABASE {name};')
        return name

    def minio(self, suffix, snapshot=None):
        volume = self.volume(f'{suffix}-objects')
        if snapshot:
            self.restore_objects(volume, snapshot)
        container = self.create_container(suffix, self.report['images']['minio']['id'],
            '-e', 'MINIO_ROOT_USER=drill', '-e', f'MINIO_ROOT_PASSWORD={self.secret}',
            '-p', '127.0.0.1::9000', '--mount', f'type=volume,src={volume},dst=/data',
            command=('server', '/data'))
        store = {'container': container, 'volume': volume}
        self.start_store(store)
        version = self.docker('exec', container, 'minio', '--version').decode().splitlines()[0]
        if not version.startswith('minio version RELEASE.2025-10-15T17-29-55Z '):
            raise AssertionError(f'unexpected pinned MinIO executable version: {version}')
        self.report.setdefault('service_versions', {})['minio'] = version
        return store

    def start_api(self, binary, database, store, label):
        self.stop_api()
        # Reserve an ephemeral loopback port, then release just before bind.
        # A collision is an explicit readiness failure, never another service.
        with socket.socket() as reserve:
            reserve.bind(('127.0.0.1', 0))
            self.api_port = reserve.getsockname()[1]
        self.api_log = (self.output / f'{label}-api.log').open('wb')
        env = {
            'PATH': os.environ.get('PATH', ''), 'HOME': os.environ.get('HOME', ''),
            'API_ADDR': f'127.0.0.1:{self.api_port}',
            'DATABASE_URL': f'postgres://postgres:{self.secret}@127.0.0.1:{self.pg_port}/{database}',
            'REDIS_URL': f'redis://127.0.0.1:{self.redis_port}',
            'MINIO_ENDPOINT': f'http://127.0.0.1:{store["port"]}',
            'MINIO_PUBLIC_ENDPOINT': f'http://127.0.0.1:{store["port"]}',
            'MINIO_ROOT_USER': 'drill', 'MINIO_ROOT_PASSWORD': self.secret,
            'MINIO_BUCKET': self.bucket, 'API_COOKIE_SECURE': 'false',
            'API_DB_MAX_CONNECTIONS': '5', 'API_RATE_AUTH_PER_MIN': '0',
            'API_RATE_API_PER_MIN': '0', 'API_RATE_MSG_PER_MIN': '0', 'API_RATE_UPLOAD_PER_HOUR': '0',
            'RUST_LOG': 'warn',
        }
        self.process = subprocess.Popen([str(binary)], env=env, stdout=self.api_log, stderr=subprocess.STDOUT)

        def ready():
            if self.process.poll() is not None:
                raise RuntimeError(f'{label} API exited; see {label}-api.log')
            return http_request(f'http://127.0.0.1:{self.api_port}/ready')[0] == 200

        self.wait(ready, f'{label} API ready')
        return self.api_port

    def stop_api(self):
        if self.process is not None:
            self.process.terminate()
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)
            self.process = None
        if self.api_log:
            self.api_log.close()
            self.api_log = None

    def rows(self, database):
        tables = self.sql(database, "SELECT tablename FROM pg_tables WHERE schemaname='public' ORDER BY tablename;").splitlines()
        return {table: json.loads(self.sql(database,
            f'SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),\'[]\'::jsonb) FROM "{table}" t;'))
            for table in tables}

    def snapshot(self, label, database, store):
        # The only writer is our API. It drains first, then both datasets stay
        # unchanged while pg_dump and the stopped MinIO volume are archived.
        self.stop_api()
        self.docker('stop', '-t', '10', store['container'])
        rows = self.rows(database)
        dump = self.docker('exec', self.pg, 'pg_dump', '-U', 'postgres', '--format=custom', database)
        helper = self.create_container(f'{label}-export', self.report['images']['postgres']['id'],
            '--network', 'none', '--read-only', '--user', '0',
            '--mount', f'type=volume,src={store["volume"]},dst=/data,readonly',
            '--entrypoint', 'tar', command=('-czf', '-', '-C', '/data', '.'))
        objects = self.docker('start', '-a', helper)
        equal(json.loads(self.docker('inspect', helper))[0]['State']['ExitCode'], 0, 'object archive helper exit')
        paths = {'database': self.output / f'{label}.pgdump', 'objects': self.output / f'{label}.minio.tgz'}
        for key, data in [('database', dump), ('objects', objects)]:
            paths[key].write_bytes(data)
            paths[key].chmod(0o600)
        manifest = {
            'database_sha256': sha(dump), 'objects_sha256': sha(objects),
            'database_bytes': len(dump), 'objects_bytes': len(objects),
            'rows_sha256': sha(json.dumps(rows, sort_keys=True).encode()),
            'minio_image_id': self.report['images']['minio']['id'],
            'postgres_image_id': self.report['images']['postgres']['id'],
            'consistency': 'own API drained; MinIO stopped; no other fixture writers',
        }
        self.report['snapshots'][label] = manifest
        (self.output / f'{label}.manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
        self.check(f'{label} paired quiesced DB/object snapshot recorded')
        return {'label': label, 'paths': paths, 'manifest': manifest, 'rows': rows}

    def verified_snapshot(self, snapshot):
        for key in ('database', 'objects'):
            equal(sha(snapshot['paths'][key].read_bytes()), snapshot['manifest'][f'{key}_sha256'],
                  f'{snapshot["label"]} {key} archive checksum')
        equal(snapshot['manifest']['minio_image_id'], self.report['images']['minio']['id'], 'same MinIO restore image')
        equal(snapshot['manifest']['postgres_image_id'], self.report['images']['postgres']['id'], 'same PostgreSQL restore image')

    def restore_objects(self, volume, snapshot):
        self.verified_snapshot(snapshot)
        helper = self.create_container(f'restore-{len(self.containers)}', self.report['images']['postgres']['id'],
            '-i', '--network', 'none',
            '--read-only', '--user', '0', '--mount', f'type=volume,src={volume},dst=/data',
            '--entrypoint', 'tar', command=('-xzf', '-', '-C', '/data'))
        self.docker('start', '-a', '-i', helper, input=snapshot['paths']['objects'].read_bytes())
        equal(json.loads(self.docker('inspect', helper))[0]['State']['ExitCode'], 0, 'object restore helper exit')

    def restore_database(self, snapshot, suffix):
        self.verified_snapshot(snapshot)
        database = self.database(suffix)
        self.docker('exec', '-i', self.pg, 'pg_restore', '-U', 'postgres', '--exit-on-error', '--no-owner',
                    '--dbname', database, input=snapshot['paths']['database'].read_bytes())
        equal(self.rows(database), snapshot['rows'], f'{suffix} every public table exact restored rows')
        self.check(f'{suffix} pg_restore exact rows including metadata/ACL/quota/migration ledger')
        return database

    def clients(self, accounts, register=False):
        clients = {}
        for name, account in accounts.items():
            client = Client(self.api_port)
            client.api('/api/auth/session')
            client.api('/api/auth/register' if register else '/api/auth/login', 'POST', account,
                       status=201 if register else 200)
            clients[name] = client
        return clients

    def upload(self, client, store, channel, filename, content_type, body, content, bind=True):
        prepared = client.api(f'/api/channels/{channel}/attachments', 'POST',
            {'filename': filename, 'content_type': content_type, 'size': len(body)}, status=201)
        url = loopback_url(prepared['upload_url'], store['port'])
        status, _, _ = http_request(url, 'PUT', body, prepared['headers'])
        equal(status, 200, 'presigned PUT into real MinIO')
        message = client.api(f'/api/channels/{channel}/messages', 'POST',
            {'content': content, 'attachment_ids': [prepared['id']]}, status=201) if bind else None
        return {'id': prepared['id'], 'channel': channel, 'message': message,
                'metadata': prepared['attachment'], 'body_sha256': sha(body), 'body_size': len(body),
                'body': body, 'put_url': url, 'put_headers': prepared['headers']}

    def download(self, client, store, attachment):
        status, headers, _ = client.raw(f'/api/attachments/{attachment["id"]}')
        equal(status, 307, 'authenticated API authorizes attachment')
        url = loopback_url(headers['Location'], store['port'])
        status, headers, data = http_request(url)
        equal(status, 200, 'real restored MinIO object readable')
        equal(sha(data), attachment['body_sha256'], 'downloaded bytes SHA256')
        equal(len(data), attachment['body_size'], 'downloaded byte length')
        equal(headers['Content-Type'], attachment['metadata']['content_type'], 'object Content-Type')
        # Even knowing the attachment's object key must not grant anonymous access.
        status, _, _ = http_request(f'http://127.0.0.1:{store["port"]}/{self.bucket}/att/{attachment["id"]}')
        equal(status, 403, 'restored bucket denies unsigned anonymous object GET')

    def verify(self, clients, store, fixtures, label):
        owner, member, outsider = (clients[key] for key in ('owner', 'member', 'outsider'))
        for attachment in fixtures:
            self.download(owner, store, attachment)
            path = f'/api/attachments/{attachment["id"]}'
            equal(outsider.raw(path)[0], 404, 'foreign account cannot download')
            equal(Client(self.api_port).raw(path)[0], 401, 'anonymous API download denied')
            if attachment['message'] is None:
                equal(member.raw(path)[0], 404, 'pending uploader-only scope restored')
            else:
                self.download(member, store, attachment)
                page = owner.api(f'/api/channels/{attachment["channel"]}/messages')
                message = next(m for m in page['messages'] if m['id'] == attachment['message']['id'])
                equal(message['attachments'], [attachment['metadata']], 'API attachment metadata/link preserved')
        channel = fixtures[0]['channel']
        member.api(f'/api/channels/{channel}/attachments', 'POST',
            {'filename': 'denied.txt', 'content_type': 'text/plain', 'size': 1}, status=403)
        self.check(f'{label} channel/DM/pending attachment hashes, metadata, private bucket and auth/ACL')

    def run(self):
        self.preflight()
        old = self.build(self.args.old_ref, 'legacy')
        current = self.build(self.args.current_ref, 'candidate')
        equal(len(self.report['binaries']['legacy']['migrations']), 9, 'legacy migration scope')
        expected = {'0010_chat_read_state.sql', '0011_message_reactions.sql'}
        if not expected.issubset(self.report['binaries']['candidate']['migrations']):
            raise AssertionError('candidate requires unchanged PR153/154 migrations')
        self.services()
        source = self.database('source')
        store = self.minio('source')
        self.start_api(old, source, store, 'legacy-seed')
        accounts = {name: {'email': f'{name}-{self.nonce}@example.test', 'password': f'Drill-{self.secret}', 'name': name}
                    for name in ('owner', 'member', 'outsider')}
        clients = self.clients(accounts, register=True)
        owner, member = clients['owner'], clients['member']
        server = owner.api('/api/servers', 'POST', {'name': 'Isolated restore drill'}, status=201)
        server_id, channel = server['id'], server['channels'][0]['id']
        invite = owner.api(f'/api/servers/{server_id}/invites', 'POST', {}, status=201)
        member.api(f'/api/invites/{invite["code"]}/join', 'POST', {}, status=200)
        dm = owner.api('/api/dms', 'POST', {'user_id': member.user['id']}, status=201)['id']
        png = base64.b64decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=')
        fixtures = [
            self.upload(owner, store, channel, 'bild.png', 'image/png', png, 'legacy searchneedle image'),
            self.upload(owner, store, channel, 'bericht.pdf', 'application/pdf', b'%PDF-1.4\nfixture\n%%EOF\n', 'legacy searchneedle document'),
            self.upload(owner, store, dm, 'dm.txt', 'text/plain', 'DM Anhang äöü\n'.encode(), 'legacy searchneedle dm'),
            self.upload(owner, store, channel, 'pending.txt', 'text/plain', b'pending uploader only\n', '', bind=False),
        ]
        owner.api(f'/api/servers/{server_id}', 'PATCH', {'member_permissions': ['send_messages']})
        self.verify(clients, store, fixtures, 'legacy seed')
        legacy = self.snapshot('legacy', source, store)
        self.start_store(store)
        self.start_api(current, source, store, 'candidate-upgrade')
        migrated = self.rows(source)
        for table in ('users', 'servers', 'server_members', 'channels', 'channel_members',
                      'messages', 'attachments', 'upload_daily_usage'):
            for before in legacy['rows'][table]:
                if not any(all(after.get(key) == value for key, value in before.items()) for after in migrated[table]):
                    raise AssertionError(f'upgrade changed a legacy {table} row')
        self.check('real migration preserves legacy users/membership/messages/attachments/quota rows')
        clients = self.clients(accounts)
        self.verify(clients, store, fixtures, 'candidate upgrade')
        owner, member, outsider = (clients[key] for key in ('owner', 'member', 'outsider'))
        for chat, fixture in [(channel, fixtures[0]), (dm, fixtures[2])]:
            page = owner.api(f'/api/channels/{chat}/messages/search?q=searchneedle')
            if not any(message['id'] == fixture['message']['id'] for message in page['messages']):
                raise AssertionError('legacy message missing from upgraded PostgreSQL search')
            member.api(f'/api/channels/{chat}/read', 'PUT', {'message_id': fixture['message']['id']})
            member.api(f'/api/messages/{fixture["message"]["id"]}/reactions/{urllib.parse.quote("❤️", safe="")}', 'PUT')
            outsider.api(f'/api/channels/{chat}/messages/search?q=searchneedle', status=404)
        fixtures.append(self.upload(owner, store, channel, 'nach-upgrade.zip', 'application/zip',
            b'PK\x03\x04current fixture bytes\x00\xff', 'current searchneedle attachment'))
        current_attachment = fixtures[-1]
        equal(http_request(current_attachment['put_url'], 'PUT', b'x' * current_attachment['body_size'],
                           current_attachment['put_headers'])[0], 200, 'same-size object corruption injected')
        self.report['corrupt_object_control'] = expected_failure(
            lambda: self.download(owner, store, current_attachment), 'downloaded bytes SHA256:')
        equal(http_request(current_attachment['put_url'], 'PUT', current_attachment['body'],
                           current_attachment['put_headers'])[0], 200, 'original object bytes put back')
        self.download(owner, store, current_attachment)
        self.check('negative control rejects same-size/Content-Type object corruption by SHA256')
        self.check('real migrated API search/read/reactions/channel+DM permissions and new attachment write')
        upgraded = self.snapshot('candidate', source, store)
        tampered = self.output / 'tampered-control.pgdump'
        tampered.write_bytes(upgraded['paths']['database'].read_bytes() + b'corrupt')
        tampered.chmod(0o600)
        tampered_snapshot = dict(upgraded, paths=dict(upgraded['paths'], database=tampered))
        self.report['corrupt_archive_control'] = expected_failure(
            lambda: self.verified_snapshot(tampered_snapshot), 'candidate database archive checksum:')
        self.check('negative control rejects a modified archive before restore')
        restored_db = self.restore_database(upgraded, 'restored')
        restored_store = self.minio('restored', upgraded)
        self.start_api(current, restored_db, restored_store, 'candidate-restored')
        restored_clients = self.clients(accounts)
        self.verify(restored_clients, restored_store, fixtures, 'candidate restore')
        for chat, fixture in [(channel, fixtures[0]), (dm, fixtures[2])]:
            history = restored_clients['owner'].api(f'/api/channels/{chat}/messages')
            message = next(m for m in history['messages'] if m['id'] == fixture['message']['id'])
            equal(message['reactions'], [{'emoji': '❤️', 'user_ids': [restored_clients['member'].user['id']]}], 'restored reaction API projection')
            reads = restored_clients['member'].api('/api/messages/unread')
            equal(next(r for r in reads if r['channel_id'] == chat)['read_message_id'], fixture['message']['id'], 'restored read API boundary')
        # A new INSERT exercises the restored immutable order sequence/default.
        newest = restored_clients['owner'].api(f'/api/channels/{channel}/messages', 'POST', {'content': 'after restore writer'}, status=201)
        if newest['created_order'] <= max(row['created_order'] for row in upgraded['rows']['messages']):
            raise AssertionError('restored creation sequence did not advance')
        self.check('restored API reaction/read state and next message sequence/default')
        pending = fixtures[3]
        uploader = restored_clients['owner'].user['id']
        quota_query = f"SELECT jsonb_build_object('reserved',sum(reserved),'consumed',sum(consumed)) FROM upload_daily_usage WHERE uploader_id='{uploader}';"
        before_quota = json.loads(self.sql(restored_db, quota_query))
        bound_pending = restored_clients['owner'].api(f'/api/channels/{channel}/messages', 'POST',
            {'content': 'bound pending upload after restore', 'attachment_ids': [pending['id']]}, status=201)
        equal(bound_pending['attachments'], [pending['metadata']], 'restored pending object can bind')
        after_quota = json.loads(self.sql(restored_db, quota_query))
        equal(after_quota, {'reserved': before_quota['reserved'] - pending['body_size'],
                            'consumed': before_quota['consumed'] + pending['body_size']},
              'restored pending quota moves reserved to consumed exactly once')
        self.check('restored pending upload binds through real API and exact quota transition')
        self.stop_api()
        # Additive SQL columns alone do not establish binary compatibility:
        # SQLx validates the applied migration ledger before binding HTTP.
        try:
            self.start_api(old, restored_db, restored_store, 'legacy-on-upgraded-control')
        except RuntimeError:
            self.stop_api()
            reason = (self.output / 'legacy-on-upgraded-control-api.log').read_text()
            if 'was previously applied but is missing in the resolved migrations' not in reason:
                raise
            self.report['old_binary_on_upgraded_schema'] = 'refused by SQLx applied migration ledger; paired legacy restore required'
        else:
            self.report['old_binary_on_upgraded_schema'] = 'old API readiness passed against upgraded schema; old snapshot rollback tested independently'
            self.stop_api()
        self.check('old binary/upgraded migration-ledger boot behavior recorded separately')
        # Negative control: upgraded DB + old object snapshot must fail for the
        # new attachment even though all its DB metadata is present.
        mismatched_store = self.minio('mismatched', legacy)
        self.start_api(current, restored_db, mismatched_store, 'mismatched-control')
        mismatch_clients = self.clients(accounts)
        self.report['mismatched_pair_control'] = expected_failure(
            lambda: self.download(mismatch_clients['owner'], mismatched_store, fixtures[-1]),
            'real restored MinIO object readable: 404 != 200')
        self.check('negative control rejects DB metadata with missing restored object bytes')
        self.stop_api()
        # An old source binary is started only against its matching old
        # migration ledger + old objectstore snapshot, not a schema simulation.
        rollback_db = self.restore_database(legacy, 'rollback')
        rollback_store = self.minio('rollback', legacy)
        self.start_api(old, rollback_db, rollback_store, 'legacy-rollback')
        rollback_clients = self.clients(accounts)
        self.verify(rollback_clients, rollback_store, fixtures[:-1], 'old binary rollback')
        rollback_clients['owner'].api(f'/api/channels/{channel}/messages', 'POST', {'content': 'old binary writes after rollback'}, status=201)
        new_id = fixtures[-1]['id']
        equal(rollback_clients['owner'].raw(f'/api/attachments/{new_id}')[0], 404, 'post-snapshot attachment absent after intentional rollback')
        self.report['rollback'] = {
            'api_binary_verified': True, 'data_snapshot': 'legacy',
            'post_snapshot_data': 'intentionally absent; snapshot rollback has an explicit recovery-point data-loss boundary',
            'full_deployment_images_verified': False,
        }
        self.check('actual frozen old API boots/authenticates/reads/writes with paired legacy restore')
        self.report['fixtures'] = [{k: item[k] for k in ('id', 'channel', 'metadata', 'body_sha256', 'body_size')}
                                   for item in fixtures]
        self.report['passed'] = True

    def cleanup(self):
        self.stop_api()
        errors = []
        for container in reversed(self.containers):
            try:
                inspected = json.loads(self.docker('inspect', container))[0]
                equal(inspected['Config']['Labels'].get(LABEL), self.nonce, 'container cleanup ownership')
                self.docker('rm', '-f', container)
            except Exception as error:
                errors.append(str(error))
        for volume in reversed(self.volumes):
            try:
                inspected = json.loads(self.docker('volume', 'inspect', volume))[0]
                equal(inspected['Labels'].get(LABEL), self.nonce, 'volume cleanup ownership')
                self.docker('volume', 'rm', volume)
            except Exception as error:
                errors.append(str(error))
        if errors:
            self.report['cleanup_errors'] = errors
            self.report['passed'] = False
        self.report['cleanup'] = ('incomplete; see cleanup_errors' if errors else
                                  'removed only nonce-labelled fixture containers and volumes')
        self.report['finished_at_utc'] = datetime.now(timezone.utc).isoformat()
        self.report['duration_seconds'] = round(time.monotonic() - self.started, 3)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True, help='new private artifact directory; must not already exist')
    parser.add_argument('--old-ref', default='v0.3.1')
    parser.add_argument('--current-ref', default='HEAD')
    parser.add_argument('--target-dir', type=Path, help='optional existing Cargo build cache; never cleaned')
    args = parser.parse_args()
    drill = Drill(args)
    try:
        drill.run()
    except Exception as error:
        drill.report['error'] = str(error)
        raise
    finally:
        try:
            drill.cleanup()
        except Exception as error:
            drill.report['passed'] = False
            drill.report.setdefault('cleanup_errors', []).append(str(error))
            raise
        finally:
            (drill.output / 'report.json').write_text(json.dumps(drill.report, indent=2) + '\n')
    print(json.dumps({'passed': drill.report['passed'], 'checks': len(drill.report['checks']), 'report': str(drill.output / 'report.json')}))
    return 0 if drill.report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
