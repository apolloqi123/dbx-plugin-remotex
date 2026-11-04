#!/usr/bin/env bash
# Build the plugin frontend and package a .dbxp for the current platform.
# Requires: node + pnpm on PATH, cargo for the Rust sidecar.
# The dbx-plugin CLI bundles its own packaging SDK; the Rust sidecar SDK used
# by this repository is vendored under shared/sdk/.
set -euo pipefail
cd "$(dirname "$0")/.."

# --fast skips the frontend typecheck/test run for local iteration; the
# package itself is still produced (and scripts/test.sh remains the gate).
FAST=0
[ "${1:-}" = "--fast" ] && FAST=1

if ! command -v pnpm >/dev/null; then
  # Node 版本单一真源是仓库根 .tool-versions（CI setup-node 使用同一版本）。
  NODE_VER="$(awk '$1 == "node" { print $2; exit }' .tool-versions 2>/dev/null)"
  NODE_BIN="$(ls -d "$HOME/.nvm/versions/node/v${NODE_VER}"*/bin 2>/dev/null | sort -V | tail -1 || true)"
  export PATH="$HOME/Library/pnpm:${NODE_BIN:+$NODE_BIN:}$PATH"
fi
# Prepend only when cargo is not already resolvable: the release CI installs a
# cargo→cargo-zigbuild wrapper ahead of the rustup shim (Linux sidecars must
# link a low glibc baseline), and an unconditional prepend here would shadow
# the wrapper and silently revert Linux builds to the runner's native glibc.
if ! command -v cargo >/dev/null 2>&1; then
  export PATH="$HOME/.cargo/bin:$PATH"
fi

# Release CI builds the target-independent UI once per plugin and stages ui/
# here as an artifact; DBX_PREBUILT_UI=1 packages it as-is instead of rerunning
# the frontend three-step on every platform job.
if [ "${DBX_PREBUILT_UI:-0}" = "1" ]; then
  if [ ! -f ui/index.html ]; then
    echo "DBX_PREBUILT_UI=1 but ui/index.html is missing; stage the CI frontend artifact first" >&2
    exit 1
  fi
  echo "==> frontend: skipped (prebuilt ui/ staged by CI)"
else
  echo "==> frontend: install + typecheck + test + build"
  # Skip install when node_modules is fresh (lockfile unchanged since); saves
  # seconds on every warm build — same trade-off ldap already makes.
  if [ ! -d frontend/node_modules ] || [ frontend/pnpm-lock.yaml -nt frontend/node_modules ]; then
    pnpm --dir frontend install --frozen-lockfile
  fi
  if [ "$FAST" = "1" ]; then
    echo "==> frontend: build (--fast: typecheck + tests skipped)"
    pnpm --dir frontend build
  else
    pnpm --dir frontend typecheck
    pnpm --dir frontend test
    pnpm --dir frontend build
  fi
fi

# dbx-plugin package always builds into a throwaway dist/.build-rust-<target>
# and deletes it afterwards, so its cargo build is cold on every run. The
# cargo-warm-bin shim (only on PATH for this command) redirects that build to
# the persistent backend/target and exposes the binary where the CLI looks, so
# packaging reuses the sidecar built by scripts/test.sh or a previous run.
# Do NOT pin RUSTUP_TOOLCHAIN here (docker builds via rust:1 + cargo-zigbuild
# lack 1.97.1); the release pipeline pins it itself.
REAL_CARGO="$(command -v cargo || true)"
[ -n "$REAL_CARGO" ] || { echo "cargo not found on PATH" >&2; exit 1; }

echo "==> package .dbxp"
# CLI 打包的内部 cargo build 会把 Rust SDK patch 覆盖到 DBX_PLUGIN_SDK_ROOT；
# 未设置时 npm 包装器注入 CLI 自带 sdk-root，vendored SDK（shared/sdk/）的本地
# 修复进不了产物——用垫片把 SDK 根指回 vendored 副本，打完做字节级反例断言。
# 垫片路径固定（backend/target/dbx-sdk-shim）：临时路径会让 Cargo 把 SDK 当成
# 新源，每次打包都重编 SDK + 插件。
export DBX_PLUGIN_SDK_ROOT="$(bash scripts/lib/sdk_root_shim.sh)"
if ! command -v dbx-plugin >/dev/null 2>&1; then
  echo "dbx-plugin CLI not found; install @dbx-app/plugin-cli or set PATH before packaging" >&2
  exit 1
fi
PATH="$PWD/scripts/lib/cargo-warm-bin:$PATH" \
  DBX_REAL_CARGO="$REAL_CARGO" \
  DBX_PLUGIN_ROOT="$PWD" \
  NO_COLOR=1 dbx-plugin package .
python3 scripts/release/verify_packaged_sdk.py

echo
echo "Artifacts:"
ls -la dist/*.dbxp dist/*.artifact.json
