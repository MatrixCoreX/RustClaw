"""Inspect the actual signed APK without modifying its bytes."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import subprocess
import zipfile


def elf_alignment(blob):
    assert blob[:4] == b"\x7fELF" and blob[5] == 1, "unsupported_elf"
    if blob[4] == 2:
        offset = struct.unpack_from("<Q", blob, 32)[0]
        size, count = struct.unpack_from("<HH", blob, 54)
        fields = "<IIQQQQQQ"
    else:
        offset = struct.unpack_from("<I", blob, 28)[0]
        size, count = struct.unpack_from("<HH", blob, 42)
        fields = "<IIIIIIII"
    loads = [struct.unpack_from(fields, blob, offset + index * size) for index in range(count)]
    return [row[-1] for row in loads if row[0] == 1]


def inspect(apk, sdk):
    build = sdk / "build-tools/36.0.0"
    def run(*args):
        return subprocess.check_output([str(arg) for arg in args], text=True, stderr=subprocess.STDOUT)
    signature = run(build / "apksigner", "verify", "--verbose", "--print-certs", apk)
    assert "Verified using v2 scheme (APK Signature Scheme v2): true" in signature
    certificate = re.search(r"Signer #1 certificate SHA-256 digest: ([0-9a-f]+)", signature).group(1)
    alignment = run(build / "zipalign", "-c", "-P", "16", "-v", "4", apk)
    assert "Verification successful" in alignment
    badging = run(build / "aapt2", "dump", "badging", apk)
    assert "name='org.agent_runtime.mobile'" in badging
    assert re.search(r"(?:minSdkVersion|sdkVersion):'26'", badging) and "targetSdkVersion:'36'" in badging
    assert "application-debuggable" not in badging
    forbidden = ["MANAGE_EXTERNAL_STORAGE", "READ_EXTERNAL_STORAGE", "WRITE_EXTERNAL_STORAGE"]
    assert not any(permission in badging for permission in forbidden)
    version = re.search(r"versionName='([^']+)'", badging).group(1)
    code = int(re.search(r"versionCode='(\d+)'", badging).group(1))
    manifest = run(build / "aapt2", "dump", "xmltree", apk, "--file", "AndroidManifest.xml")
    elements, stack = [], []
    for line in manifest.splitlines():
        indent = len(line) - len(line.lstrip())
        element = re.match(r"\s*E: (\S+)", line)
        if element:
            while stack and stack[-1][0] >= indent:
                stack.pop()
            item = {"element": element.group(1)}
            elements.append(item)
            stack.append((indent, item))
        elif stack and "A: http://schemas.android.com/apk/res/android:" in line:
            key, value = line.split("/android:", 1)[1].split("=", 1)
            stack[-1][1][key.split("(")[0]] = value.split(" (Raw:")[0].strip('"')
    app = next(item for item in elements if item['element'] == 'application')
    assert not any(item['element']=='instrumentation' for item in elements)
    assert app['allowBackup'] == 'false' and app['fullBackupContent'] == 'false'
    assert app['resizeableActivity'] == 'true' and app['usesCleartextTraffic'] == 'false'
    for name in ['WalletActivity', 'CompanionActivity', 'VaultService']:
        component = next(item for item in elements if item.get('name') == 'org.agent_runtime.mobile.' + name)
        assert component['exported'] == 'false', name
        if name == 'VaultService':
            assert component['process'] == ':asset_vault' and component['stopWithTask'] == 'true'
    native = []
    with zipfile.ZipFile(apk) as archive:
        names = archive.namelist()
        assert not any(part in name for name in names for part in [".p12", ".keystore", "wallet-linux-v", "test-only-vault"])
        for name in names:
            if name.endswith('.dex'):
                code_bytes=archive.read(name)
                assert not any(test in code_bytes for test in [b'UiHarnessTest', b'NativeSecurityTest', b'WalletRoundTripTest']), 'test_bridge_in_release'
        for name in names:
            if name.startswith("lib/") and name.endswith(".so"):
                blob = archive.read(name)
                abi = name.split('/')[1]
                loads = elf_alignment(blob)
                assert loads and all(value >= (16384 if abi in {"arm64-v8a", "x86_64"} else 4096) for value in loads), name
                native.append({"file": name, "sha256": hashlib.sha256(blob).hexdigest(), "load_alignments": loads})
    abis = sorted({item['file'].split('/')[1] for item in native})
    assert abis == ["arm64-v8a", "armeabi-v7a", "x86_64"], abis
    return {"schema_version": 1, "version": version, "version_code": code, "file": apk.name,
            "sha256": hashlib.file_digest(apk.open('rb'), 'sha256').hexdigest(), "bytes": apk.stat().st_size,
            "certificate_sha256": certificate, "min_sdk": 26, "target_sdk": 36, "abis": abis,
            "native_libraries": native, "checks": ["apk_v2_signature", "zip_16kb_alignment", "elf_16kb_alignment_64bit",
                "release_not_debuggable", "no_test_instrumentation_or_bridge", "no_broad_storage_permission", "no_test_backups_or_release_signing_key",
                "backup_disabled", "private_wallet_components", "separate_wallet_process", "resizable_screens"],
            "limits": ["Package inspection is not a substitute for runtime and device testing."]}


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('apk', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = inspect(args.apk.resolve(), Path(os.environ['ANDROID_HOME']))
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps(result, ensure_ascii=False, indent=2))
