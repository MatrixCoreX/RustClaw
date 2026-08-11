#!/usr/bin/env python3
"""Ratchet channel copy catalogs and production message-key coverage."""
from __future__ import annotations

import argparse
import re
import tempfile
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable


REPO_ROOT = Path(__file__).resolve().parents[1]


@dataclass(frozen=True)
class CatalogSpec:
    name: str
    locales: tuple[str, ...]
    minimum_key_count: int


CATALOG_SPECS = (
    CatalogSpec("channel-common", ("en-US", "zh-CN", "ja", "ko"), 10),
    CatalogSpec("telegramd", ("en-US", "zh-CN"), 49),
    CatalogSpec("wechatd", ("en-US", "zh-CN"), 24),
    CatalogSpec("feishud", ("en-US", "zh-CN"), 10),
    CatalogSpec("larkd", ("en-US", "zh-CN"), 10),
    CatalogSpec("whatsapp-cloud", ("en-US", "zh-CN"), 6),
    CatalogSpec("whatsapp-webd", ("en-US", "zh-CN"), 8),
)

PRODUCTION_SOURCE_ROOTS = (
    "crates/claw-core/src",
    "crates/clawd/src",
    "crates/telegramd/src",
    "crates/wechatd/src",
    "crates/whatsappd/src",
    "crates/whatsapp_webd/src",
    "crates/feishud/src",
    "crates/larkd/src",
    "crates/skills",
    "optional_skills",
    "external_skills",
    "services/wa-web-bridge",
    "configs/channel_commands.toml",
)

PRODUCTION_SOURCE_SUFFIXES = (".rs", ".js", ".mjs", ".cjs", ".ts", ".toml")
UNSAFE_PUBLIC_PLACEHOLDERS = frozenset(
    {"error", "err", "body", "detail", "path", "token", "key", "cookie", "stack"}
)

PLACEHOLDER_RE = re.compile(r"\{([A-Za-z][A-Za-z0-9_]*)\}")
MESSAGE_KEY_RE = re.compile(
    r'''["']('''
    r'''(?:common\.[A-Za-z0-9_.-]+|'''
    r'''(?:channel|telegram|wechat|feishu|lark|whatsapp_cloud|whatsapp_web)'''
    r'''\.(?:msg|error|menu|progress|log|task|notice)\.[A-Za-z0-9_.-]+)'''
    r''')["']'''
)


def flatten_dict(value: object, prefix: str = "") -> dict[str, str]:
    if not isinstance(value, dict):
        raise ValueError("dict table is missing")
    flattened: dict[str, str] = {}
    for raw_key, child in value.items():
        key = str(raw_key)
        qualified = f"{prefix}.{key}" if prefix else key
        if isinstance(child, dict):
            flattened.update(flatten_dict(child, qualified))
        elif isinstance(child, str):
            flattened[qualified] = child
        else:
            raise ValueError(f"non-string copy value: {qualified}")
    return flattened


def load_catalog(path: Path, expected_locale: str) -> tuple[dict[str, str], list[str]]:
    relative = path.as_posix()
    if not path.is_file():
        return {}, [f"catalog_missing:{relative}"]
    try:
        document = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        return {}, [f"catalog_parse_failed:{relative}:{error}"]
    findings: list[str] = []
    if document.get("locale") != expected_locale:
        findings.append(
            f"locale_mismatch:{relative}:expected={expected_locale}:"
            f"actual={document.get('locale')!r}"
        )
    try:
        entries = flatten_dict(document.get("dict"))
    except ValueError as error:
        return {}, findings + [f"catalog_invalid:{relative}:{error}"]
    for key, text in sorted(entries.items()):
        normalized = text.strip()
        if not normalized:
            findings.append(f"copy_empty:{relative}:{key}")
        if normalized == key or normalized.startswith("message_key="):
            findings.append(f"machine_copy_leak:{relative}:{key}")
    return entries, findings


def production_source_files(root: Path, source_roots: Iterable[str]) -> Iterable[Path]:
    for relative in source_roots:
        source_root = root / relative
        if source_root.is_file():
            if source_root.suffix in PRODUCTION_SOURCE_SUFFIXES:
                yield source_root
            continue
        if not source_root.is_dir():
            continue
        for path in sorted(source_root.rglob("*")):
            if not path.is_file() or path.suffix not in PRODUCTION_SOURCE_SUFFIXES:
                continue
            if (
                path.name.endswith(("_tests.rs", "tests.rs", ".test.js", ".test.ts"))
                or "tests" in path.parts
                or "node_modules" in path.parts
            ):
                continue
            yield path


def referenced_message_keys(root: Path, source_roots: Iterable[str]) -> set[str]:
    referenced: set[str] = set()
    for path in production_source_files(root, source_roots):
        referenced.update(MESSAGE_KEY_RE.findall(path.read_text(encoding="utf-8")))
    return referenced


def validate(
    root: Path,
    specs: Iterable[CatalogSpec] = CATALOG_SPECS,
    source_roots: Iterable[str] = PRODUCTION_SOURCE_ROOTS,
) -> tuple[list[str], dict[str, int], int]:
    findings: list[str] = []
    all_keys: set[str] = set()
    counts: dict[str, int] = {}
    for spec in specs:
        catalogs: dict[str, dict[str, str]] = {}
        for locale in spec.locales:
            path = root / "configs/i18n" / f"{spec.name}.{locale}.toml"
            entries, catalog_findings = load_catalog(path, locale)
            findings.extend(catalog_findings)
            catalogs[locale] = entries
        reference_locale = spec.locales[0]
        reference = catalogs[reference_locale]
        counts[spec.name] = len(reference)
        if len(reference) < spec.minimum_key_count:
            findings.append(
                f"key_count_regressed:{spec.name}:minimum={spec.minimum_key_count}:"
                f"actual={len(reference)}"
            )
        reference_keys = set(reference)
        all_keys.update(reference_keys)
        for locale, entries in catalogs.items():
            for key, text in sorted(entries.items()):
                unsafe = sorted(
                    set(PLACEHOLDER_RE.findall(text)) & UNSAFE_PUBLIC_PLACEHOLDERS
                )
                if unsafe:
                    findings.append(
                        f"unsafe_public_placeholder:{spec.name}:{locale}:{key}:"
                        f"placeholders={unsafe}"
                    )
        for locale in spec.locales[1:]:
            localized = catalogs[locale]
            localized_keys = set(localized)
            for key in sorted(reference_keys - localized_keys):
                findings.append(f"localized_key_missing:{spec.name}:{locale}:{key}")
            for key in sorted(localized_keys - reference_keys):
                findings.append(f"reference_key_missing:{spec.name}:{locale}:{key}")
            for key in sorted(reference_keys & localized_keys):
                expected = sorted(PLACEHOLDER_RE.findall(reference[key]))
                actual = sorted(PLACEHOLDER_RE.findall(localized[key]))
                if expected != actual:
                    findings.append(
                        f"placeholder_mismatch:{spec.name}:{locale}:{key}:"
                        f"expected={expected}:actual={actual}"
                    )

    referenced = referenced_message_keys(root, source_roots)
    for key in sorted(referenced - all_keys):
        findings.append(f"production_message_key_missing:{key}")
    for key in sorted(all_keys - referenced):
        findings.append(f"unused_catalog_key:{key}")
    return findings, counts, len(referenced)


def write_fixture_catalog(path: Path, locale: str, entries: dict[str, str]) -> None:
    lines = [f'locale = "{locale}"', "", "[dict]"]
    for key, value in entries.items():
        escaped = value.replace("\\", "\\\\").replace('"', '\\"')
        lines.append(f'"{key}" = "{escaped}"')
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def run_self_test() -> int:
    spec = CatalogSpec("channel-common", ("en-US", "zh-CN"), 2)
    with tempfile.TemporaryDirectory(prefix="channel-i18n-completeness-") as tmp:
        root = Path(tmp)
        write_fixture_catalog(
            root / "configs/i18n/channel-common.en-US.toml",
            "en-US",
            {
                "common.safe_generic_error": "Try again, {name}.",
                "channel.error.delivery_failed": "Delivery failed.",
            },
        )
        write_fixture_catalog(
            root / "configs/i18n/channel-common.zh-CN.toml",
            "zh-CN",
            {
                "common.safe_generic_error": "请重试。",
                "channel.error.delivery_failed": "message_key=channel.error.delivery_failed",
            },
        )
        source = root / "crates/clawd/src/main.rs"
        source.parent.mkdir(parents=True)
        source.write_text(
            'let _ = "channel.error.provider_unavailable";\n'
            'let _ = "common.safe_generic_error";\n',
            encoding="utf-8",
        )
        bridge = root / "services/wa-web-bridge/index.js"
        bridge.parent.mkdir(parents=True)
        bridge.write_text(
            'const key = "common.safe_generic_error";\n', encoding="utf-8"
        )
        findings, _, _ = validate(root, (spec,), ("crates/clawd/src",))
        expected_kinds = {
            "machine_copy_leak",
            "placeholder_mismatch",
            "production_message_key_missing",
            "unused_catalog_key",
        }
        actual_kinds = {finding.split(":", 1)[0] for finding in findings}
        if actual_kinds != expected_kinds:
            print(f"SELF_TEST_FAIL findings={findings}")
            return 1

        write_fixture_catalog(
            root / "configs/i18n/channel-common.en-US.toml",
            "en-US",
            {
                "common.safe_generic_error": "Try again, {error}.",
                "channel.error.delivery_failed": "Delivery failed.",
            },
        )
        findings, _, referenced = validate(
            root,
            (spec,),
            ("crates/clawd/src", "services/wa-web-bridge"),
        )
        if not any(
            finding.startswith("unsafe_public_placeholder:") for finding in findings
        ) or referenced != 2:
            print(
                "SELF_TEST_FAIL unsafe_or_js_scan "
                f"findings={findings} referenced={referenced}"
            )
            return 1

        repaired = {
            "common.safe_generic_error": "请{name}重试。",
            "channel.error.delivery_failed": "无法投送。",
        }
        write_fixture_catalog(
            root / "configs/i18n/channel-common.en-US.toml",
            "en-US",
            {
                "common.safe_generic_error": "Try again, {name}.",
                "channel.error.delivery_failed": "Delivery failed.",
            },
        )
        write_fixture_catalog(
            root / "configs/i18n/channel-common.zh-CN.toml", "zh-CN", repaired
        )
        source.write_text(
            'let _ = "channel.error.delivery_failed";\n', encoding="utf-8"
        )
        findings, counts, referenced = validate(
            root, (spec,), ("crates/clawd/src", "services/wa-web-bridge")
        )
        if findings or counts != {"channel-common": 2} or referenced != 2:
            print(
                "SELF_TEST_FAIL repaired "
                f"findings={findings} counts={counts} referenced={referenced}"
            )
            return 1
    print("CHANNEL_I18N_COMPLETENESS_SELF_TEST ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return run_self_test()

    findings, counts, referenced = validate(REPO_ROOT)
    if findings:
        print("CHANNEL_I18N_COMPLETENESS_CHECK failed")
        for finding in findings:
            print(f"- {finding}")
        return 1
    counts_text = ",".join(f"{name}:{count}" for name, count in counts.items())
    print(
        "CHANNEL_I18N_COMPLETENESS_CHECK ok "
        f"catalogs={len(counts)} keys={counts_text} referenced={referenced}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
