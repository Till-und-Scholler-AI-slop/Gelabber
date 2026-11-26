"""Checks an update signature as the native app does (desktop/ui/src/platform/update.rs, `verify`).

desktop-native.yml signs a candidate's installer once the release is tagged
and runs none of the built code, so the app's own check (the test
`the_installer_built_here`) cannot run there. This repeats it: the minisign
signature by the base64 key in desktop/packaging/updater.pub, its trusted
comment, and the `version:` field the app requires.

Usage: update_signature.py FILE SIGNATURE_FILE PUBKEY_FILE VERSION
"""
import base64
import hashlib
import sys

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey


def lines(text_b64):
    """The minisign text inside a base64 file, line by line."""
    return base64.b64decode(text_b64.strip()).decode().splitlines()


def verify(data, signature_b64, pubkey_b64, version):
    """Raises ValueError unless the signature is updater.pub's, for `version`."""
    key = base64.b64decode(lines(pubkey_b64)[1])
    signature_lines = lines(signature_b64)
    if len(signature_lines) < 4 or not signature_lines[2].startswith('trusted comment: '):
        raise ValueError('not a minisign signature')
    signed = base64.b64decode(signature_lines[1])
    trusted = signature_lines[2][len('trusted comment: '):]
    global_signature = base64.b64decode(signature_lines[3])
    algorithm, key_id, signature = signed[:2], signed[2:10], signed[10:]
    if key[:2] != b'Ed' or len(key) != 42 or len(signature) != 64:
        raise ValueError('malformed key or signature')
    if key_id != key[2:10]:
        raise ValueError('signed with another key than updater.pub')
    if algorithm == b'ED':
        message = hashlib.blake2b(data, digest_size=64).digest()
    elif algorithm == b'Ed':
        message = data  # legacy, which the app accepts too
    else:
        raise ValueError(f'unknown signature algorithm {algorithm!r}')
    public = Ed25519PublicKey.from_public_bytes(key[10:])
    try:
        public.verify(signature, message)
        public.verify(global_signature, signature + trusted.encode())
    except InvalidSignature as error:
        raise ValueError('the signature does not match the file and updater.pub') from error
    fields = dict(field.split(':', 1) for field in trusted.split('\t') if ':' in field)
    if 'version' not in fields:
        raise ValueError('signed without a version')
    if fields['version'].lstrip('v') != version.lstrip('v'):
        raise ValueError(f'signed for {fields["version"]}, not {version}')
    return trusted


def main(argv):
    path, signature_path, pubkey_path, version = argv
    with open(path, 'rb') as file:
        data = file.read()
    with open(signature_path) as file:
        signature = file.read()
    with open(pubkey_path) as file:
        pubkey = file.read()
    try:
        trusted = verify(data, signature, pubkey, version)
    except ValueError as error:
        sys.exit(f'{path}: {error}')
    print(f'{path}: signed by {pubkey_path} ({trusted})')


if __name__ == '__main__':
    main(sys.argv[1:])
