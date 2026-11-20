"""Versioned authenticated backup objects; never place recovery keys in a vault."""
import hashlib
import hmac
import os
from pathlib import Path
import secrets
import struct
import zlib

CHUNK_BYTES = 4 * 1024 * 1024
MAX_MANIFEST_BYTES = 64 * 1024 * 1024
MAGIC = b'ARV1'


def load_key(path, create=False):
    if not hasattr(os,'getuid'):raise RuntimeError('vault_platform_unsupported')
    path = Path(path)
    if create and not path.exists():
        path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, 'O_NOFOLLOW', 0), 0o600)
        with os.fdopen(fd, 'wb') as stream:
            stream.write(secrets.token_bytes(32)); stream.flush(); os.fsync(stream.fileno())
    try:meta = path.lstat()
    except FileNotFoundError as error:raise RuntimeError('vault_key_missing') from error
    if path.is_symlink() or not path.is_file() or meta.st_uid != os.getuid() or meta.st_mode & 0o077:
        raise RuntimeError('vault_key_not_private')
    value = path.read_bytes()
    if len(value) != 32: raise RuntimeError('vault_key_invalid')
    return value


class VaultCipher:
    def __init__(self, key):
        if len(key) != 32: raise RuntimeError('vault_key_invalid')
        try:
            from cryptography.hazmat.primitives import hashes
            from cryptography.hazmat.primitives.kdf.hkdf import HKDF
            from cryptography.hazmat.primitives.ciphers.aead import AESGCM
        except ImportError as error:
            raise RuntimeError('vault_crypto_dependency_missing') from error
        derive = lambda label: HKDF(algorithm=hashes.SHA256(), length=32, salt=None, info=b'runtime-backup-vault-v1/' + label).derive(key)
        self.identity = derive(b'object-id')
        self.objects = AESGCM(derive(b'object-aead'))
        self.manifests = AESGCM(derive(b'manifest-aead'))

    def object_id(self, data):
        return hmac.new(self.identity, data, hashlib.sha256).hexdigest()

    @staticmethod
    def seal(cipher, data, aad):
        nonce = secrets.token_bytes(12)
        return MAGIC + nonce + cipher.encrypt(nonce, data, MAGIC + aad)

    @staticmethod
    def open(cipher, data, aad):
        if len(data) < 32 or data[:4] != MAGIC: raise RuntimeError('vault_cipher_format_invalid')
        try: return cipher.decrypt(data[4:16], data[16:], MAGIC + aad)
        except Exception as error: raise RuntimeError('vault_authentication_failed') from error

    def encrypt_object(self, raw):
        if len(raw) > CHUNK_BYTES: raise RuntimeError('vault_chunk_too_large')
        identity = self.object_id(raw)
        compressed = zlib.compress(raw, level=3)
        packed = bytes([int(len(compressed) < len(raw))]) + struct.pack('>I', len(raw))
        packed += compressed if len(compressed) < len(raw) else raw
        return identity, self.seal(self.objects, packed, identity.encode('ascii'))

    def decrypt_object(self, identity, blob):
        packed = self.open(self.objects, blob, identity.encode('ascii'))
        if len(packed) < 5: raise RuntimeError('vault_chunk_invalid')
        expected = struct.unpack('>I', packed[1:5])[0]
        if expected > CHUNK_BYTES or packed[0] not in (0, 1): raise RuntimeError('vault_chunk_invalid')
        if packed[0]:
            decoder = zlib.decompressobj()
            raw = decoder.decompress(packed[5:], CHUNK_BYTES + 1)
            if not decoder.eof or decoder.unused_data or decoder.unconsumed_tail: raise RuntimeError('vault_chunk_invalid')
        else: raw = packed[5:]
        if len(raw) != expected or not hmac.compare_digest(self.object_id(raw), identity): raise RuntimeError('vault_chunk_digest_mismatch')
        return raw

    def encrypt_manifest(self, value):
        if len(value) > MAX_MANIFEST_BYTES: raise RuntimeError('vault_manifest_too_large')
        return self.seal(self.manifests, value, b'manifest')

    def decrypt_manifest(self, blob):
        if len(blob) > MAX_MANIFEST_BYTES + 32: raise RuntimeError('vault_manifest_too_large')
        return self.open(self.manifests, blob, b'manifest')
