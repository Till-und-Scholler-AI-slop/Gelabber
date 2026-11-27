"""compose.yaml as Docker Compose renders it (`docker compose config`), for
the tests. Needs the Docker CLI, not a daemon."""
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
# The examples ship without passwords (#188); an operator fills them in.
SECRETS = {key: f'test-{key.lower()}' for key in (
    'POSTGRES_PASSWORD', 'MINIO_ROOT_PASSWORD', 'MINIO_APP_PASSWORD', 'TURN_PASSWORD')}


def config(settings=None, file='compose.yaml'):
    """The rendered configuration with the secrets and `settings` in .env."""
    values = {**SECRETS, **(settings or {})}
    with tempfile.TemporaryDirectory() as tmp:
        env = Path(tmp) / 'test.env'
        env.write_text(''.join(f'{key}={value}\n' for key, value in values.items()))
        # The shell's environment wins over --env-file (CI exports
        # MINIO_ROOT_PASSWORD for its own MinIO); test the file alone.
        environ = {key: value for key, value in os.environ.items()
                   if not key.endswith('PASSWORD') and key not in values}
        result = subprocess.run(['docker', 'compose', '--project-directory', str(ROOT),
                                 '--env-file', str(env), '-f', str(ROOT / file),
                                 'config', '--format', 'json'],
                                capture_output=True, text=True, env=environ)
    if result.returncode != 0:
        raise AssertionError(f'docker compose config failed: {result.stderr.strip()}')
    return json.loads(result.stdout)
