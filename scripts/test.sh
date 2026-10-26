#!/usr/bin/env bash
# Full verification suite for the SSH/SFTP plugin.
#
#   scripts/test.sh                # everything available on this machine
#   scripts/test.sh --skip-host    # skip the host-worktree install pipeline
#
# Steps: repo identity/manifest contract -> backend unit tests -> frontend
# typecheck/test/build -> sidecar release build -> .dbxp package -> MCP stdio
# smoke -> host install-pipeline integration test (PluginPackageInstaller +
# MCP bridge over a real sidecar).
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$(pwd)"

SKIP_HOST=0
[ "${1:-}" = "--skip-host" ] && SKIP_HOST=1

if ! command -v node >/dev/null 2>&1 || ! command -v pnpm >/dev/null 2>&1; then
  # Node 版本单一真源是仓库根 .tool-versions（CI setup-node 使用同一版本）。
  NODE_VER="$(awk '$1 == "node" { print $2; exit }' .tool-versions 2>/dev/null)"
  NODE_BIN="$(ls -d "$HOME/.nvm/versions/node/v${NODE_VER}"*/bin 2>/dev/null | sort -V | tail -1 || true)"
  export PATH="$HOME/Library/pnpm:${NODE_BIN:+$NODE_BIN:}$PATH"
fi
export PATH="$HOME/.cargo/bin:$PATH"

# 用一次性数据目录跑全套：known_hosts / Quick Sudo 配置等状态不落用户真实
# 插件数据，主机密钥挑战类用例也可重复——否则测试容器一旦重建（host key
# 变更），remembered 记录会把连接按 MITM 防护直接拒掉。
export DBX_PLUGIN_DATA_DIR="$(mktemp -d "${TMPDIR:-/tmp}/dbx-ssh-test-data.XXXXXX")"

# 与 UI smoke 共用的门禁开关：DBX_SMOKE_STRICT=1 时 live 环境缺失（Docker/
# 测试容器）与未注册 RPC 按失败处理；未设置时保持本地 SKIP 全绿的既有语义。
# 显式 export 让 Python live smokes 与 .mjs 读到同一份值（.mjs 自读 env）。
export DBX_SMOKE_STRICT="${DBX_SMOKE_STRICT:-0}"

echo "==> repo identity/manifest contract"
# 与 ci.yml / release.yml 的 validate job 同一步：manifest ↔ .dbx-store.json
# 的 id/版本/权限/本地化一致，backend 身份与必需路径未漂移。纯 stdlib、
# 无网络；放在最前面，store/清单回归秒失败，不必等整轮构建。
python3 scripts/verify/validate_repo.py

echo "==> backend unit tests"
node scripts/verify/connection-forms/verify.mjs
# 方法注册四路比对（backend 分派 ↔ mock 桩 ↔ 前端 invoke ↔ PROTOCOL 文档）：
# 分派臂/桩/调用点三方不一致是硬门禁，文档滞后 report-only。
node scripts/verify/mock_host_registry_check.mjs
# 单测密闭：宿主桥端口读 DBX_APP_DATA_DIR/mcp-bridge-port，未设时回落真实
# app 数据目录——开发机上 DBX 运行中会让 read_only 稳定性等用例读到真实连接
# （首次命中/二次抖动即漂移），甚至经 ensure_app_bridge 的 open -a 兜底拉起
# DBX。钉到一次性空目录（与 CI 无运行 DBX 同态），启动命令置 no-op。
DBX_APP_DATA_DIR="$DBX_PLUGIN_DATA_DIR" DBX_APP_LAUNCH_CMD="true" \
  cargo test --manifest-path backend/Cargo.toml

echo "==> dead-code warning gate"
# 未接线的功能会先以 dead_code 警告形态暴露（案例：remembered 审批 /
# ssh_terminal_input 的安全函数写完却没接进路由）。零容忍：
# cargo build 出现任何 dead_code 警告即失败，避免新功能静默脱接。
#
# 不要用 `cargo build ... | grep -q ...` 做判定：grep -q 命中即
# 退出，cargo 继续写输出会收到 SIGPIPE（141），pipefail 下整条管道非零、
# if 被判假，门禁静默放行。这里只构建一次并落盘，再对文件做匹配。
#
# cargo 对已是最新的单元不重放警告（热 target 只打印 "Finished"），门禁
# 会静默放行。先 clean 掉本包的产物强制重编（依赖保留缓存，不是冷构建）。
deadcode_log="$(mktemp -t dbx-deadcode.XXXXXX)"
trap 'rm -f "${deadcode_log:-}"' EXIT
cargo clean -p dbx-plugin-sshx --manifest-path backend/Cargo.toml
if ! cargo build --manifest-path backend/Cargo.toml >"$deadcode_log" 2>&1; then
  cat "$deadcode_log" >&2
  echo "FAIL: cargo build failed (output above)" >&2
  exit 1
fi
# dead_code 的三种形态：未使用项 "never used"、未读字段 "is never read"、
# 未构造的结构体/枚举变体 "is never constructed"。模式抽成变量，判定与
# 打印共用，避免两处漂移。
DEADCODE_PATTERN='never used|never read|never constructed'
if grep -Eq "$DEADCODE_PATTERN" "$deadcode_log"; then
  echo "FAIL: dead-code warnings present (an unwired feature or leftover code?):" >&2
  grep -E -B1 "$DEADCODE_PATTERN" "$deadcode_log" >&2
  exit 1
fi
echo "  no dead-code warnings"

echo "==> frontend sandbox-confirm lint (SSH-H1 regression gate)"
# 宿主沙箱 iframe 无 allow-modals，window.confirm 恒返回 false——危险确认
# （覆盖/删除/强杀）必须走 lib/confirmDialog 应用内弹窗；出现即失败。
if grep -rn "window\.confirm(" frontend/src >/dev/null 2>&1; then
  echo "FAIL: window.confirm( found (sandbox iframe has no allow-modals, always false):" >&2
  grep -rn "window\.confirm(" frontend/src >&2
  exit 1
fi
echo "  no window.confirm calls"

echo "==> frontend typecheck + tests + build"
[ -d frontend/node_modules ] || pnpm --dir frontend install --frozen-lockfile
pnpm --dir frontend typecheck
pnpm --dir frontend test
pnpm --dir frontend build

echo "==> sidecar release build"
cargo build --release --manifest-path backend/Cargo.toml

echo "==> package .dbxp"
# 与 build.sh 一致：把 CLI 内部 cargo build 重定向到热的 backend/target。
# 不重定向时 CLI 用一次性 dist/.build-rust-<triple> 暂存目录——整树冷编译，
# 且会被并发/下一次打包的清理踩掉（表现为写 .fingerprint 时 ENOENT）。
export CARGO_TARGET_DIR="$ROOT/backend/target"
# CLI 打包的内部 cargo build 会把 Rust SDK patch 覆盖到 DBX_PLUGIN_SDK_ROOT；
# 未设置时 npm 包装器注入 CLI 自带 sdk-root，vendored SDK（shared/sdk/）的本地
# 修复进不了产物——用垫片把 SDK 根指回 vendored 副本，打完做字节级反例断言。
if command -v dbx-plugin >/dev/null 2>&1; then
  export DBX_PLUGIN_SDK_ROOT="$(bash scripts/lib/sdk_root_shim.sh)"
  NO_COLOR=1 dbx-plugin package .
  python3 scripts/release/verify_packaged_sdk.py
else
  echo "SKIP: dbx-plugin CLI unavailable; install @dbx-app/plugin-cli to run package verification"
fi

echo "==> MCP stdio smoke"
python3 scripts/smoke/smoke_mcp.py --binary backend/target/release/dbx-plugin-sshx

# 本地终端冒烟：驱动本机登录 shell（spawn/注入标记/回显/关闭），不需要
# SSH 容器；二进制缺失时脚本自 SKIP。放 docker 小节之外，任何机器都跑。
echo "==> local terminal smoke"
python3 scripts/smoke/smoke_local_terminal.py --binary backend/target/release/dbx-plugin-sshx

# 本机 Docker 冒烟（容器管理底部分屏的 target:"local" 面）：不依赖
# dbx-ssh-test 测试容器，有本机 Docker 守护进程即可，故与下面的 live 小节分开。
# 两个脚本都自带跳过语义，无 Docker 的机器保持全绿：
#   - smoke_docker_local.py：无守护进程时 list 退化为 available:false，只跑
#     形状断言并通过；但 sidecar 二进制缺失会直接抛错（sidecar_client），
#     故仅在 release 产物存在时运行。
#   - smoke_docker_files.py：docker CLI/守护进程/可用本地镜像任一缺失即打印
#     SKIP 并 exit 0；只创建一次性临时容器与临时目录、退出时清理，从不 pull
#     镜像、不改动已有容器/镜像。无镜像属环境条件而非回归，因此这里不随
#     DBX_SMOKE_STRICT 升级为失败（live 小节的 strict 语义不变）。
echo "==> local Docker smokes (container manager: list/logs/files)"
export DBX_PLUGIN_SIDECAR="$ROOT/backend/target/release/dbx-plugin-sshx"
if [ -x "$DBX_PLUGIN_SIDECAR" ]; then
  python3 scripts/smoke/smoke_docker_local.py
else
  echo "SKIP: smoke_docker_local.py (sidecar binary missing)"
fi
python3 scripts/smoke/smoke_docker_files.py

# Live-container smokes + perf baseline: each script SKIPs its cases when the
# test container is unavailable; the whole section is skipped when docker or
# the dbx-ssh-test container is absent so the suite stays green everywhere.
if command -v docker >/dev/null 2>&1 && docker ps --format '{{.Names}}' 2>/dev/null | grep -qx 'dbx-ssh-test'; then
  export DBX_PLUGIN_SIDECAR="$ROOT/backend/target/release/dbx-plugin-sshx"
  echo "==> live smoke: smoke_test.py"
  python3 scripts/smoke/smoke_test.py
  echo "==> live smoke: smoke_fs_test.py"
  python3 scripts/smoke/smoke_fs_test.py
  echo "==> live smoke: smoke_batch3_test.py"
  python3 scripts/smoke/smoke_batch3_test.py
  echo "==> live smoke: smoke_sudo_otp_test.py"
  python3 scripts/smoke/smoke_sudo_otp_test.py
  echo "==> live smoke: smoke_trigger_test.py"
  python3 scripts/smoke/smoke_trigger_test.py
  # Agent terminal approval/deny/timeout safety matrix (26 scenarios). The
  # script SKIPs when its SSH target is unreachable; under DBX_SMOKE_STRICT=1
  # that SKIP is a hard failure and the run must execute at least three
  # approval/deny/timeout/sudo cases, so the gate cannot silently omit them.
  echo "==> live smoke: e2e_agent_terminal.py"
  python3 scripts/smoke/e2e_agent_terminal.py
  echo "==> perf baseline (P-SSH §3; ±20% thresholds documented in TEST_MATRIX)"
  python3 scripts/smoke/perf_baseline_test.py
else
  # strict 门禁下，Docker/测试容器缺失是环境损坏，不能与「测试通过」混淆。
  if [ "$DBX_SMOKE_STRICT" = "1" ]; then
    echo "FAIL: live smokes require docker and the dbx-ssh-test container (DBX_SMOKE_STRICT=1)" >&2
    exit 1
  fi
  echo "SKIP: live smokes + perf baseline (docker or dbx-ssh-test container unavailable)"
fi

# 协议矩阵真机层（telnet/serial Docker 真机覆盖，scripts/docker/protocol-matrix/）：
# 对真实 telnetd 跑 IAC/NAWS/自动登录，容器内 Linux sidecar 跑 socat 虚拟串口
# + X/Y/ZMODEM 回环（macOS 伪终端 ENOTTY 缺口的补位层）。Linux 冷构建较慢，
# 默认 opt-in（DBX_PROTOCOL_MATRIX=1），SKIP 时套件保持全绿。
if [ "${DBX_PROTOCOL_MATRIX:-0}" = "1" ]; then
  if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
    echo "==> protocol matrix (docker telnet/serial real-device layer)"
    bash scripts/docker/protocol-matrix/run_matrix.sh
  else
    echo "SKIP: protocol matrix (docker unavailable)"
  fi
else
  echo "SKIP: protocol matrix (set DBX_PROTOCOL_MATRIX=1 to opt in)"
fi

# Mock UI walkthrough (COLLECT-FINAL suggestion 1): headless-Chrome anchor
# assertions + screenshot. Self-gating — SKIPs (exit 0) without playwright-core
# (/tmp/dbx-ui-mock) or system Chrome; a real failure fails the suite.
echo "==> mock UI walkthrough"
node scripts/smoke/smoke_ui_mock.mjs

if [ "$SKIP_HOST" = 0 ]; then
  HOST="${DBX_HOST_WORKTREE:-}"
  if [ -n "$HOST" ] && [ -d "$HOST" ]; then
    echo "==> host install-pipeline integration (installer + MCP bridge)"
    PACKAGE="$ROOT/$(ls -t dist/*.dbxp | head -1)"
    # The bridge test tracks in-progress MCP plugin-tools work; if it does not
    # compile yet, skip it instead of failing the whole suite. Once it
    # compiles it must pass.
    if (cd "$HOST" && cargo test -p dbx-mcp --test plugin_tools_bridge --no-run >/dev/null 2>&1); then
      (cd "$HOST" && DBX_SSH_SFTP_PACKAGE="$PACKAGE" cargo test -p dbx-mcp --test plugin_tools_bridge)
    else
      echo "SKIP: plugin_tools_bridge does not compile (MCP plugin-tools bridge WIP not integrated)"
    fi
  else
    echo "==> host install-pipeline integration skipped (standalone checkout; set DBX_HOST_WORKTREE to opt in)"
  fi
else
  echo "==> host install-pipeline integration skipped (--skip-host)"
fi

echo
echo "all green"
