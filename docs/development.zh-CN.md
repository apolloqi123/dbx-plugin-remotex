# 开发指南

本文面向修改 RemoteX、运行测试和构建发布包的开发者。

## 环境

版本以仓库根 `.tool-versions` 为准：

| 工具 | 版本 |
| --- | --- |
| Rust | `1.97.1` |
| rustfmt | `1.97.1` |
| clippy | `1.97.1` |
| Node.js | `26.12.1` |
| pnpm | `11.24.0`（`frontend/package.json`） |
| Go | `1.27`（仅部分工具脚本使用） |

打包还需要：

- `dbx-plugin` CLI（`@dbx-app/plugin-cli`）
- `zip` / `unzip`
- Docker（可选，用于 live smoke 与协议矩阵）

## 仓库结构

```
.
├── backend/          Rust sidecar
├── frontend/         Vue 3 UI
├── shared/           前后端共享协议与 vendored Rust SDK
├── assets/           插件图标
├── manifest.json     插件清单
├── dbx-plugin.toml   打包配置
├── scripts/          构建、测试、验证、发布脚本
├── docs/             文档
└── dist/             打包产物
```

## 常用命令

### 前端

```bash
pnpm --dir frontend install
pnpm --dir frontend typecheck
pnpm --dir frontend test
pnpm --dir frontend build
```

- `typecheck`：`vue-tsc --noEmit`
- `test`：Vitest
- `build`：Vite 构建到 `ui/`

运行单个测试文件：

```bash
pnpm --dir frontend test src/features/recording/components/RecordingsPanel.spec.ts
```

### 后端

```bash
cargo test --manifest-path backend/Cargo.toml
cargo build --manifest-path backend/Cargo.toml
cargo clippy --manifest-path backend/Cargo.toml --all-targets
cargo fmt --manifest-path backend/Cargo.toml -- --check
```

macOS 沙箱环境下并行启动 PTY 可能不稳定；`scripts/test.sh` 默认使用
`--test-threads=1`。可用 `BACKEND_TEST_THREADS` 覆盖。

### 全量验证

```bash
scripts/test.sh
scripts/test.sh --skip-host   # 跳过宿主安装流水线
```

`scripts/test.sh` 依次执行：

1. `scripts/verify/validate_repo.py`：清单、版本、权限、本地化、身份一致性
2. `scripts/verify/connection-forms/verify.mjs`：连接表单字段
3. `scripts/verify/mock_host_registry_check.mjs`：RPC 分派 ↔ mock ↔ 前端调用 ↔ 协议文档
4. 后端单元测试
5. 前端类型检查、测试、构建
6. sidecar release 构建
7. MCP stdio smoke 与本地终端 smoke
8. 打包 `.dbxp`
9. 宿主安装流水线（未跳过时）

可选环境变量：

| 变量 | 作用 |
| --- | --- |
| `DBX_SMOKE_STRICT=1` | live 环境缺失按失败处理，而不是跳过 |
| `DBX_PROTOCOL_MATRIX=1` | 运行 Telnet/串口 Docker 真机矩阵 |
| `BACKEND_TEST_THREADS=N` | 后端测试并发数 |
| `DBX_PLUGIN_DATA_DIR=...` | 使用一次性插件数据目录 |

### 打包

```bash
scripts/build.sh          # 前端类型检查 + 测试 + 构建 + sidecar + .dbxp
scripts/build.sh --fast   # 跳过前端类型检查与测试
```

产物：

- `dist/<plugin>-<version>-<platform>-<arch>.dbxp`
- `dist/<plugin>-<version>-<platform>-<arch>.artifact.json`

安装到本机 DBX：

```bash
scripts/install.sh
scripts/install.sh --no-restart
```

### 发布前检查

```bash
scripts/release/package_and_verify.sh
python3 scripts/release/check_candidates.py dist/*.dbxp
```

`package_and_verify.sh` 会从 `.dbxp` 中解出 sidecar，对打包后的真实字节跑 smoke，
避免“测的是 target 目录、发的是另一个二进制”。

## 修改 RPC

新增或重命名 RPC 方法时，以下四处必须同时更新：

1. `backend/src/app/rpc/<domain>.rs` 的 `match` 分派
2. `frontend/src/app/mock/host/<domain>.ts` 的 mock 实现
3. `frontend/src` 中的调用点
4. `docs/architecture/PROTOCOL.zh-CN.md` 的方法表

然后运行：

```bash
node scripts/verify/mock_host_registry_check.mjs
```

脚本会报告：

- 后端有、mock 缺（失败）
- mock/白名单中有、后端不存在（失败）
- 前端调用了后端不存在的方法（失败）
- 协议文档缺条目（警告）

协议文档的文本也参与校验：表格第一列的反引号方法名会被提取。不要在表格第一列
写非方法名的反引号内容。

## 修改连接表单

连接字段由 `manifest.json` 的 `connection-provider` 贡献点声明。修改后运行：

```bash
node scripts/verify/connection-forms/verify.mjs
```

它会检查字段可见性、必填条件、默认值与本地化是否自洽。

## 代码组织约束

- `backend/src/structure_tests.rs` 会检查模块边界；不要把协议栈依赖带进 UI 层模块。
- 新增功能优先放在 `features/<name>/`，跨协议能力不要塞进 `sessions/ssh/`。
- 前端按 feature 分目录：组件、composable、纯函数分开放。
- 纯逻辑（排序、解析、计划生成）放在 `lib/`，便于单测。
- 错误必须转成用户可见的状态；不要静默吞掉二进制通道或会话错误。
- 涉及凭据的代码不得把值写日志、错误字符串或测试快照。

## 测试策略

| 层 | 方式 |
| --- | --- |
| 纯函数 | Rust `#[cfg(test)]` / Vitest |
| RPC handler | 后端单测 + mock host |
| UI 交互 | Vue Test Utils，断言可访问名称与键盘行为 |
| 终端帧 | 序号、缺口、回放、洪峰回归测试 |
| 真实协议 | `scripts/smoke/*` 对 Docker 容器或本机服务 |
| 打包 | 从 `.dbxp` 解包后跑 smoke |

新增行为至少覆盖：

- 正常路径
- 失败路径与错误文案
- 边界值（空值、超长、并发、断线）
- 安全门禁（只读、路径、危险命令）

## 文档

- 用户文档在 `docs/`，中文为主；README 提供中英双语入口。
- 协议变更必须同步 `docs/architecture/PROTOCOL.zh-CN.md`。
- 文档中不硬编码插件技术标识，使用 `<plugin-id>` 占位。
- 截图放在 `docs/media/`，使用描述性文件名。

## 提交前检查清单

- [ ] `pnpm --dir frontend typecheck`
- [ ] `pnpm --dir frontend test`
- [ ] `cargo test --manifest-path backend/Cargo.toml`
- [ ] `node scripts/verify/mock_host_registry_check.mjs`
- [ ] `python3 scripts/verify/validate_repo.py`
- [ ] 打包并安装，手动走一遍受影响的主流程
- [ ] 协议或行为变更已更新文档

## 已知环境限制

- 某些 macOS 环境缺少 `librustc_driver` 动态库时，`cargo fmt` 可能无法启动；
  这是本机 rustfmt 安装问题，不是代码格式失败。
- live smoke 需要 Docker 与测试容器；不可用时脚本会 `SKIP`，离线通过不代表
  live 连接通过。
- 协议矩阵（Telnet/串口）默认不运行，需 `DBX_PROTOCOL_MATRIX=1`。
