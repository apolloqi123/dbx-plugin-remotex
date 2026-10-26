#!/usr/bin/env python3
"""Validate standalone SSH plugin identity and package-relative paths."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SEMVER = re.compile(r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$")
# Keep in sync with scripts/verify/connection-forms/verify.mjs.
EXPECTED_LOCALES = ("en", "es", "it", "ja", "pt-BR", "zh-CN", "zh-TW")
# PluginMetadata::new(<id>, env!("CARGO_PKG_VERSION")): raw id or centralized const.
METADATA_IDENTITY = re.compile(
    r'PluginMetadata::new\(\s*(?:"(?P<literal>[^"]*)"|(?P<constant>PLUGIN_ID))'
    r'\s*,\s*env!\("CARGO_PKG_VERSION"\)'
)
PLUGIN_ID_CONST = re.compile(
    r"(?m)^[ \t]*pub(?:\([^)]*\))?\s+const\s+PLUGIN_ID\s*:\s*&\s*(?:'static\s+)?str"
    r'\s*=\s*"([^"]*)"\s*;'
)


def fail(message: str) -> None:
    raise SystemExit(f"FAIL: {message}")


def load_json(path: Path, label: str) -> dict:
    """Load a JSON object, rejecting duplicate keys at any nesting level."""
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as exc:
        fail(f"cannot read {label}: {exc}")

    def reject_duplicate_keys(pairs: list[tuple[str, object]]) -> dict:
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f"duplicate JSON key: {key!r}")
            result[key] = value
        return result

    try:
        value = json.loads(text, object_pairs_hook=reject_duplicate_keys)
    except ValueError as exc:  # includes json.JSONDecodeError
        fail(f"invalid {label}: {exc}")
    if not isinstance(value, dict):
        fail(f"{label} must be a JSON object")
    return value


def validate_icon(value: object, label: str) -> None:
    """Require a repository-relative regular file that cannot escape ROOT."""
    if not isinstance(value, str) or not value:
        fail(f"{label} icon must be a non-empty string")
    if value.startswith(("http://", "https://")):
        return
    relative = Path(value)
    if relative.is_absolute():
        fail(f"{label} icon must be repository-relative: {value}")
    probe = ROOT
    for part in relative.parts:
        probe = probe / part
        if probe.is_symlink():
            fail(f"{label} icon must not be a symlink: {value}")
    candidate = ROOT / relative
    if not candidate.is_file():
        fail(f"{label} icon does not exist or is not a regular file: {value}")
    try:
        candidate.resolve(strict=True).relative_to(ROOT.resolve())
    except (OSError, ValueError):
        fail(f"{label} icon escapes repository root: {value}")


def validate_backend_identity(plugin_id: str) -> None:
    """Require bootstrap PluginMetadata to use the manifest id, literal or PLUGIN_ID."""
    try:
        bootstrap_rs = (ROOT / "backend/src/app/bootstrap.rs").read_text(encoding="utf-8")
    except OSError as exc:
        fail(f"cannot read backend/src/app/bootstrap.rs: {exc}")
    calls = list(METADATA_IDENTITY.finditer(bootstrap_rs))
    if not calls:
        fail(
            "backend initialization identity is missing or stale: expected "
            f'PluginMetadata::new("{plugin_id}", env!("CARGO_PKG_VERSION")) or '
            'PluginMetadata::new(PLUGIN_ID, env!("CARGO_PKG_VERSION")) in '
            "backend/src/app/bootstrap.rs"
        )
    uses_constant = False
    for call in calls:
        literal_id = call.group("literal")
        if literal_id is None:
            uses_constant = True
        elif literal_id != plugin_id:
            fail(
                "backend initialization identity is stale: "
                f'PluginMetadata::new uses "{literal_id}", expected "{plugin_id}"'
            )
    if not uses_constant:
        return
    try:
        plugin_ids_rs = (ROOT / "backend/src/core/plugin_ids.rs").read_text(encoding="utf-8")
    except OSError as exc:
        fail(f"cannot read backend/src/core/plugin_ids.rs for PLUGIN_ID: {exc}")
    constants = PLUGIN_ID_CONST.findall(plugin_ids_rs)
    if not constants:
        fail(
            "backend initialization uses PLUGIN_ID but "
            "backend/src/core/plugin_ids.rs does not define `pub const PLUGIN_ID: &str`"
        )
    for constant_id in constants:
        if constant_id != plugin_id:
            fail(
                f"backend PLUGIN_ID is {constant_id!r}, expected manifest id {plugin_id!r} "
                "in backend/src/core/plugin_ids.rs"
            )


def main() -> int:
    manifest_path = ROOT / "manifest.json"
    manifest = load_json(manifest_path, "manifest.json")
    if manifest.get("manifest_version") != 1:
        fail("manifest_version must be 1")
    if manifest.get("id") != "io.dbx.sshx":
        fail(f"manifest id is {manifest.get('id')!r}, expected io.dbx.sshx")
    version = manifest.get("version")
    if not isinstance(version, str) or not SEMVER.fullmatch(version):
        fail(f"invalid manifest version: {version!r}")
    for field in ("source", "homepage"):
        value = manifest.get(field, "")
        if not isinstance(value, str) or not value.startswith(("http://", "https://")):
            fail(f"{field} must be an HTTP(S) URL")

    entrypoint = manifest.get("entrypoints", {}).get("backend", {})
    if entrypoint.get("executable") != "bin/dbx-plugin-sshx":
        fail("backend executable must be bin/dbx-plugin-sshx")
    if manifest.get("entrypoints", {}).get("ui", {}).get("entry") != "ui/index.html":
        fail("UI entry must be ui/index.html")

    toml = (ROOT / "dbx-plugin.toml").read_text(encoding="utf-8")
    if 'directory = "backend"' not in toml or 'binary = "dbx-plugin-sshx"' not in toml:
        fail("dbx-plugin.toml backend identity/path is stale")

    store_path = ROOT / ".dbx-store.json"
    if not store_path.is_file():
        fail("missing .dbx-store.json")
    store = load_json(store_path, ".dbx-store.json")

    manifest_permissions = set(manifest.get("permissions", []))
    store_permissions = set(store.get("permissions", []))
    unexpected_permissions = store_permissions - manifest_permissions
    if unexpected_permissions:
        fail(
            ".dbx-store.json declares permissions absent from manifest: "
            + ", ".join(sorted(unexpected_permissions))
        )
    missing_permissions = manifest_permissions - store_permissions
    if missing_permissions:
        fail(
            ".dbx-store.json is missing manifest permissions: "
            + ", ".join(sorted(missing_permissions))
        )

    shared_identity_fields = [
        field
        for field in ("id", "name", "version", "icon", "source", "homepage")
        if field in manifest and field in store
    ]
    for field in shared_identity_fields:
        if store[field] != manifest[field]:
            fail(
                f".dbx-store.json {field} is {store[field]!r}, "
                f"manifest has {manifest[field]!r}"
            )

    validate_icon(manifest.get("icon"), "manifest")
    for contribution in manifest.get("contributions", []):
        if isinstance(contribution, dict) and "icon" in contribution:
            label = f"manifest contribution {contribution.get('id', '<unknown>')}"
            validate_icon(contribution["icon"], label)
    icon = store.get("icon")
    if "icon" in store:
        validate_icon(icon, ".dbx-store.json")

    manifest_locales = manifest.get("localizations", {})
    if not isinstance(manifest_locales, dict):
        fail("manifest localizations must be an object")
    missing_locales = [locale for locale in EXPECTED_LOCALES if locale not in manifest_locales]
    if missing_locales:
        fail(
            "manifest localizations missing canonical locales: "
            + ", ".join(missing_locales)
        )
    for locale, values in store.get("localizations", {}).items():
        expected_name = manifest_locales.get(locale, {}).get("name")
        if expected_name is not None and values.get("name") != expected_name:
            fail(
                f".dbx-store.json localization {locale} name is "
                f"{values.get('name')!r}, manifest has {expected_name!r}"
            )

    contribution_ids = [
        contribution["id"]
        for contribution in manifest.get("contributions", [])
        if isinstance(contribution, dict) and "id" in contribution
    ]
    if not contribution_ids:
        fail("manifest declares no contribution ids")
    if len(contribution_ids) != len(set(contribution_ids)):
        fail("manifest declares duplicate contribution ids")
    expected_ids = set(contribution_ids)
    for locale, values in manifest_locales.items():
        keys = list(values.get("contributions", {}).keys())
        if len(keys) != len(set(keys)):
            fail(f"manifest localization {locale} has duplicate contribution keys")
        if set(keys) != expected_ids:
            missing = sorted(expected_ids - set(keys))
            extra = sorted(set(keys) - expected_ids)
            fail(
                f"manifest localization {locale} contribution keys mismatch: "
                f"missing={missing}, extra={extra}"
            )

    cargo = (ROOT / "backend/Cargo.toml").read_text(encoding="utf-8")
    if not re.search(r'(?m)^name\s*=\s*"dbx-plugin-sshx"\s*$', cargo):
        fail("backend Cargo package name does not match manifest")
    if not re.search(rf'(?m)^version\s*=\s*"{re.escape(version)}"\s*$', cargo):
        fail("backend Cargo version does not match manifest")
    lock = (ROOT / "backend/Cargo.lock").read_text(encoding="utf-8")
    if not re.search(
        rf'(?m)^name = "dbx-plugin-sshx"\r?\nversion = "{re.escape(version)}"\s*$',
        lock,
    ):
        fail("backend Cargo.lock version does not match manifest")
    changelog = (ROOT / "CHANGELOG.md").read_text(encoding="utf-8")
    if not re.search(rf"(?m)^## \[{re.escape(version)}\](?:\s|$)", changelog):
        fail(f"CHANGELOG has no section for {version}")
    validate_backend_identity(manifest["id"])

    required = [
        "assets/plugin.svg", "frontend/package.json", "backend/Cargo.toml",
        "scripts/test.sh", "scripts/verify/connection-forms/verify.mjs",
        "shared/frontend/binaryEvent.ts", "shared/frontend/editorTheme.ts",
        "shared/frontend/themeSync.ts", "shared/sdk/rust/dbx-plugin-sdk/src/lib.rs",
    ]
    for relative in required:
        if not (ROOT / relative).exists():
            fail(f"missing required path: {relative}")

    print(
        f"PASS store contract: {len(store_permissions)} permissions match manifest; "
        f"shared identity fields match manifest ({', '.join(shared_identity_fields)})"
    )
    print(
        f"PASS manifest localizations: {len(manifest_locales)} locales cover all "
        f"{len(contribution_ids)} contribution ids"
    )
    print(f"PASS repository identity: {manifest['id']} {version}; standalone paths and vendored SDK present")
    return 0


if __name__ == "__main__":
    sys.exit(main())
