#!/usr/bin/env python3
"""Local PostgreSQL upgrade/backup/restore drill using fresh, owned databases.

Runs only against an explicitly named local test container. Never reads or
restores an application database. The PostgreSQL container must already exist.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import uuid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--container', required=True)
    parser.add_argument('--user', default='postgres')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if not re.fullmatch(r'gelabber-[a-z0-9-]+-test-postgres', args.container):
        parser.error('use a dedicated gelabber-*-test-postgres container')
    if not re.fullmatch(r'[a-zA-Z_][a-zA-Z0-9_]*', args.user):
        parser.error('invalid PostgreSQL fixture user')
    if os.environ.get('DOCKER_HOST'):
        parser.error('DOCKER_HOST overrides are not supported for this local drill')
    context = json.loads(subprocess.check_output(['docker', 'context', 'inspect'], text=True))[0]
    endpoint = context['Endpoints']['docker']['Host']
    if not endpoint.startswith('unix://'):
        parser.error('a local Unix Docker endpoint is required')
    inspected = json.loads(subprocess.check_output(['docker', 'inspect', args.container], text=True))[0]
    if not inspected['State']['Running'] or not inspected['Config']['Image'].startswith('postgres:'):
        parser.error('the dedicated test container must run PostgreSQL')
    root = Path(__file__).resolve().parents[1]
    migrations = sorted((root / 'api/migrations').glob('*.sql'))
    legacy = [path for path in migrations if int(path.name.split('_', 1)[0]) <= 9]
    additive = [path for path in migrations if int(path.name.split('_', 1)[0]) > 9]
    if len(legacy) != 9 or not additive:
        parser.error('expected the shipped nine migrations and additive v0.4 migrations')
    suffix = uuid.uuid4().hex
    source, restored = f'gb_upgrade_{suffix}', f'gb_restore_{suffix}'
    owned = []
    report = {
        'schema': 1, 'passed': False, 'scope': 'isolated schema/data drill; no production or object-store restore',
        'postgres_image_id': inspected['Image'],
        'source_revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(),
        'migration_sha256': {path.name: hashlib.sha256(path.read_bytes()).hexdigest() for path in migrations},
        'checks': [],
    }

    def sql(database, statement):
        return subprocess.check_output(
            ['docker', 'exec', '-i', args.container, 'psql', '-X', '-v', 'ON_ERROR_STOP=1',
             '-U', args.user, '-d', database, '-At'], input=statement, text=True,
        ).strip()

    def equal(database, statement, expected, label):
        actual = sql(database, statement)
        if actual != expected:
            raise AssertionError(f'{label}: {actual!r} != {expected!r}')
        report['checks'].append(label)

    try:
        for database in [source, restored]:
            sql('postgres', f'CREATE DATABASE {database};')
            owned.append(database)
        for path in legacy:
            sql(source, path.read_text())
        user, server, channel, first, second = [str(uuid.uuid4()) for _ in range(5)]
        sql(source, f"""
            INSERT INTO users(id,email,name,password_hash) VALUES('{user}','upgrade@example.test','Upgrade','unused');
            INSERT INTO servers(id,name,owner_id) VALUES('{server}','Upgrade','{user}');
            INSERT INTO server_members(server_id,user_id) VALUES('{server}','{user}');
            INSERT INTO channels(id,server_id,name,kind) VALUES('{channel}','{server}','upgrade','text');
            INSERT INTO messages(id,channel_id,author_id,content,created_at)
            VALUES('{second}','{channel}','{user}','newer searchneedle','2026-10-05T01:00:00Z'),
                  ('{first}','{channel}','{user}','older searchneedle','2026-10-05T00:00:00Z');
        """)
        for path in additive:
            sql(source, path.read_text())
        equal(source, 'SELECT string_agg(content,\',\' ORDER BY created_order) FROM messages;',
              'older searchneedle,newer searchneedle', 'legacy creation order backfilled')
        equal(source, f"SELECT created_order FROM messages WHERE id='{second}';", '2', 'deterministic cursor backfill')
        sql(source, f"""
            INSERT INTO message_reactions(message_id,user_id,emoji) VALUES('{first}','{user}','❤️');
            INSERT INTO channel_read_state(user_id,channel_id,membership_at,message_at,message_id,created_order)
            SELECT '{user}','{channel}',sm.joined_at,m.created_at,m.id,m.created_order
            FROM server_members sm,messages m WHERE sm.server_id='{server}' AND sm.user_id='{user}' AND m.id='{second}';
        """)
        dump = subprocess.check_output(['docker', 'exec', args.container, 'pg_dump', '-U', args.user, '--format=custom', source])
        report['backup_sha256'] = hashlib.sha256(dump).hexdigest()
        report['backup_bytes'] = len(dump)
        subprocess.run(['docker', 'exec', '-i', args.container, 'pg_restore', '-U', args.user,
                        '--exit-on-error', '--no-owner', '--dbname', restored], input=dump, check=True)
        # Compare complete ordered rows, not only their counts.
        for table, order in [('messages', 'created_order'), ('message_reactions', 'message_id,user_id,emoji'),
                             ('channel_read_state', 'user_id,channel_id')]:
            statement = f'SELECT row_to_json(t) FROM (SELECT * FROM {table} ORDER BY {order}) t;'
            equal(restored, statement, sql(source, statement), f'{table} exact restored rows')
        equal(restored, "SELECT count(*) FROM messages WHERE to_tsvector('simple',content) @@ websearch_to_tsquery('simple','searchneedle');",
              '2', 'restored full-text search')
        # Simulate an old application's INSERT/SELECT footprint after additive
        # upgrade: omitted new columns must still default, not reject writes.
        equal(restored, f"INSERT INTO messages(channel_id,author_id,content) VALUES('{channel}','{user}','old writer after restore') RETURNING created_order;",
              '3\nINSERT 0 1', 'restored sequence accepts legacy writer')
        equal(restored, f"SELECT count(*) FROM messages WHERE channel_id='{channel}' AND created_order>(SELECT created_order FROM channel_read_state WHERE user_id='{user}' AND channel_id='{channel}');",
              '1', 'restored read boundary excludes earlier messages')
        equal(restored, f"DELETE FROM messages WHERE id='{first}'; SELECT count(*) FROM message_reactions;",
              'DELETE 1\n0', 'restored foreign-key cascade')
        report['passed'] = True
    except Exception as error:
        report['error'] = str(error)
        raise
    finally:
        # Database names are generated locally and are never supplied by callers.
        for database in reversed(owned):
            try:
                sql('postgres', f'DROP DATABASE {database};')
            except Exception as error:
                report.setdefault('cleanup_errors', []).append(str(error))
                report['passed'] = False
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'passed': report['passed'], 'checks': report['checks']}, indent=2))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
