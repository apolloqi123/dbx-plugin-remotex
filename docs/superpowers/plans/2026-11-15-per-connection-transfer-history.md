# Per-Connection Transfer History Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Make the SFTP transfer-history panel show and clear only rows owned by the current saved SSH connection, while preserving live progress and legacy behavior.

**Architecture:** Use the stable `connectionId` as the ownership key end to end: every persisted transition carries it, query/clear accept it, and the frontend passes a getter so reconnect/session changes do not leak rows across connections. Keep the existing global 200-row ring buffer; filter scoped views before sorting/truncating. Rows without a `connectionId` are legacy/unattributed and stay hidden from scoped views, but an explicitly unscoped legacy query can still see them.

**Tech Stack:** Vue 3 + TypeScript composables, Rust sidecar JSON-RPC, serde_json, Vitest, Rust `cargo test`, Python smoke test, project mock host + registry validator.

**Spec:** `docs/superpowers/specs/2026-11-15-per-connection-transfer-history-design.md`

## Global Constraints

- Use `connectionId` (saved connection id), never `sessionId` (ephemeral), as the history ownership key.
- Preserve `MAX_HISTORY = 200` as a global cap; do not create per-connection files.
- Preserve the 0600 atomic write and `STORAGE_VERSION = 1`; no storage migration.
- Keep `sessionId?` filter backward-compatible for old MCP/smoke callers; `connectionId?` adds a second, independently applicable filter.
- Local save rows get `connectionId: "local"` so they never appear in an SSH connection's view.
- All seven i18n locales already require aligned keys; this plan adds no i18n keys.
- Do not commit, push, package, or modify docs/README/CHANGELOG in this plan.
- After code edits: run `codegraph sync`, then `codegraph status`.
- All frontend calls to backend methods must be literal strings so `scripts/verify/mock_host_registry_check.mjs` can verify them.

## Review Focus

1. **Legacy rows without `connectionId`:** a scoped query must return an empty list rather than leaking global rows; Task 3 pins this.
2. **Terminal transitions that omit `connectionId`:** `record_transfer` must resolve it from the session or persisted start row; Task 1 pins both.
3. **Live tasks whose session is no longer registered:** they must not appear in another connection's scoped view; Task 1/3 pin empty `connectionId`.
4. **Clear while another connection has rows:** only matching persisted/in-memory rows disappear; Task 3 pins this.
5. **No `connectionId` in the workbench context (RemoteX local mode):** the frontend must not send an empty filter that shows global rows; Task 2 pins `connectionId: ""` → local scope.

---

## File Structure

- `backend/src/sessions/ssh/transfer/lifecycle.rs`: enrich terminal transitions and scope in-memory clear.
- `backend/src/sessions/ssh/transfer/query.rs`: scope query/clear by `connectionId`.
- `backend/src/features/transfer/history.rs`: update `clear_history` to retain other scopes; add tests.
- `backend/src/filesystem/local/downloads.rs`: tag local saves.
- `backend/src/app/rpc/transfers.rs`: parse/validate `connectionId`.
- `frontend/src/features/transfers/composables/useTransferHistory.ts`: pass scope from a getter.
- `frontend/src/app/workbench/setup/step6-connections.ts`: inject `connectionId`.
- `frontend/src/app/mock/host/transfers.ts`, `frontend/src/app/mockDbxHost.spec.ts`: mirror scoped mock contract.
- `backend/src/sessions/ssh/tests/transfer.rs`, `scripts/smoke/smoke_fs_test.py`, `docs/architecture/PROTOCOL.zh-CN.md`: contract tests/docs.

### Task 1: Attach `connectionId` to every terminal transition

**Files:**
- Modify: `backend/src/sessions/ssh/transfer/lifecycle.rs`
- Test: `backend/src/sessions/ssh/tests/transfer.rs`

**Interfaces:**
- Consumes: `SshRuntime::session_connection_id(&self, session_id: &str) -> String` (async).
- Produces: `pub(crate) async fn record_transfer(&self, task: Value)`; `pub(crate) fn record_transfer_start(...) -> ()`; `pub(crate) fn persist_transfer_record(&self, task: &Value)` unchanged.

- [x] **Step 1: Write the failing test** in `backend/src/sessions/ssh/tests/transfer.rs`: a terminal transition built without `connectionId` but with a registered `sessionId` through `runtime.sessions` must persist `connectionId: "c1"`; a transition for `sessionId: "s-gone"` with no prior row must persist `connectionId: ""`.
- [x] **Step 2: Run the targeted test** with `cargo test --manifest-path backend/Cargo.toml transfer_history_persists_connection_id -- --nocapture`. Expected: FAIL because `record_transfer` is synchronous and lacks enrichment.
- [x] **Step 3: Make `record_transfer` async and add `connection_id` resolution**: if the incoming `task["connectionId"]` is a nonempty string keep it; else await `self.session_connection_id(session_id)`; if still empty, read the persisted row for the same `taskId` and inherit its `connectionId`. Insert the resolved nonempty value into `task` before the existing in-memory and disk writes. Keep `record_transfer_start` synchronous (its signature already receives `connection_id`).
- [x] **Step 4: Add `.await` at every `record_transfer(...)` caller** found by `rtk rg -n 'record_transfer\(' backend/src`; keep `record_transfer_start` call sites unchanged.
- [x] **Step 5: Make `cleanup_session_transfers` capture `connection_id` once** via `session_connection_id(session_id).await` before removing registries; include it in both failed-transition payloads, and retain in-memory history rows only when their scoped `connectionId` equals the captured value. For legacy rows without a key, retain by `sessionId` so disconnect still clears them.
- [x] **Step 6: Run targeted backend tests** with `cargo test --manifest-path backend/Cargo.toml transfer_history -- --nocapture`. Expected: PASS, including the new enrichment test.

### Task 2: Scoped query/clear RPC and frontend ownership

**Files:**
- Modify: `backend/src/app/rpc/transfers.rs`, `backend/src/sessions/ssh/transfer/query.rs`, `backend/src/features/transfer/history.rs`, `backend/src/filesystem/local/downloads.rs`
- Modify: `frontend/src/features/transfers/composables/useTransferHistory.ts`, `frontend/src/app/workbench/setup/step6-connections.ts`
- Test: `backend/src/sessions/ssh/tests/transfer.rs`, `backend/src/features/transfer/history.rs`, `frontend/src/app/mockDbxHost.spec.ts`

**Interfaces:**
- Consumes: Task 1's enriched persisted rows.
- Produces:
  - `pub async fn transfer_history_query(&self, session_id: Option<&str>, connection_id: Option<&str>, limit: usize) -> Result<Value, String>`
  - `pub async fn clear_transfer_history(&self, connection_id: Option<&str>) -> Result<(), String>`
  - `pub fn clear_history(data_dir: &Path, connection_id: Option<&str>) -> Result<(), String>`
  - `pub(crate) fn build_transfer_history(&self, session_id: Option<&str>, connection_id: Option<&str>, limit: usize, connection_id_for: &impl Fn(&str) -> String) -> Result<Value, String>`
  - `useTransferHistory({ ..., connectionId: () => string })`

- [x] **Step 1: Write failing Rust tests**: `build_transfer_history(None, Some("c1"), 50, ...)` returns only `c1` and excludes a legacy row with no `connectionId`; `clear_history(&dir, Some("c1"))` leaves `c2` and local rows; `save_local_file` writes `connectionId: "local"`.
- [x] **Step 2: Run targeted tests** with `cargo test --manifest-path backend/Cargo.toml transfer_history -- --nocapture`. Expected: FAIL with signature/arity errors.
- [x] **Step 3: Implement `clear_history(data_dir, connection_id)`**: when `Some(scope)`, load rows, retain rows whose `connectionId` is not that exact scope, and `save_rows`; when `rows.is_empty()`, remove the file; when `None`, keep the existing whole-file removal. Use the same atomic 0600 write.
- [x] **Step 4: Implement scoped `build_transfer_history`**: after merging live rows and before sorting/truncating, apply `session_id` filter if present and `connection_id` filter if present; exact string equality, missing key does not match. Make `clear_transfer_history` async, accept `connection_id`, retain in-memory rows whose `connectionId` differs, and pass the scope to `clear_history`.
- [x] **Step 5: Wire `backend/src/app/rpc/transfers.rs`**: parse optional nonempty `connectionId` with `params.get("connectionId").and_then(Value::as_str).filter(...)`; pass it to `transfer_history_query` and `clear_transfer_history().await`; keep `sessionId?`, `limit` clamp (1..=200), and response shape unchanged.
- [x] **Step 6: Tag local save rows**: add `"connectionId": "local"` to both `record_transition` payloads in `backend/src/filesystem/local/downloads.rs`.
- [x] **Step 7: Add frontend `connectionId` getter**: change `useTransferHistory` options to `connectionId: () => string`; in `refreshTransferHistory`, include `connectionId: options.connectionId()` only when nonempty; when empty, include `connectionId: "local"`; use the same scope for `sftp/transfer/history/clear`. Pass `connectionId: () => connectionId.value` from `step6-connections.ts`.
- [x] **Step 8: Run targeted Rust and frontend tests**: `cargo test --manifest-path backend/Cargo.toml transfer_history -- --nocapture` and `cd frontend && pnpm vitest run src/app/mockDbxHost.spec.ts`. Expected: PASS; update mock host first if the frontend contract test fails for the new parameter.

### Task 3: Mock, smoke, docs, and full verification

**Files:**
- Modify: `frontend/src/app/mock/host/transfers.ts`, `frontend/src/app/mockDbxHost.spec.ts`, `scripts/smoke/smoke_fs_test.py`, `docs/architecture/PROTOCOL.zh-CN.md`
- Test: all files above

**Interfaces:**
- Consumes: Task 2's scoped RPC contract.
- Produces: mock host with `const clearedTransferScopes = new Set<string>()` and per-scope filtering; smoke assertions for two connection scopes.

- [x] **Step 1: Write failing frontend mock tests**: default context returns rows with `connectionId`; a second connection scope returns `[]`; clear for one scope leaves the other scope intact. Update `?err=transferHistory` and `limit` assertions to pass `connectionId`.
- [x] **Step 2: Run `cd frontend && pnpm vitest run src/app/mockDbxHost.spec.ts`**. Expected: FAIL with global clear/single-scope fixture.
- [x] **Step 3: Implement mock host scoping**: replace the global boolean with a `Set<string>` or `Record<string, boolean>` keyed by `connectionId`; filter entries by exact `connectionId` before `slice(0, limit)`; include at least one `connectionId: "visual-other-connection"` row and one `connectionId: "local"` row so leakage is visible; clear marks only the requested scope. Keep fixture rows already tagged with `context.connectionId`.
- [x] **Step 4: Update Python smoke** at `case_transfer_history` around line 1526: pass `{"limit": 50, "connectionId": connection_id}`; add a second query with `"connectionId": "smoke-fs-other-connection"` asserting no `taskId` match; add a clear-by-scope assertion that a synthetic `c2` row is preserved while `connection_id` rows are removed, then restore baseline. Do not run the Docker smoke test unless the local SSH fixture is available; report it as skipped otherwise.
- [x] **Step 5: Update `docs/architecture/PROTOCOL.zh-CN.md`** table descriptions and the `sftp/transfer/history` detail paragraph: document `connectionId?`, exact-match filtering, scoped clear, legacy unattributed rows hidden from scoped views, and `connectionId: "local"` for local saves.
- [x] **Step 6: Run full verification**:
  - `node scripts/verify/mock_host_registry_check.mjs`
  - `python3 -I scripts/verify/validate_repo.py`
  - `cd frontend && pnpm typecheck && pnpm test`
  - `cargo test --manifest-path backend/Cargo.toml`
  - `codegraph sync && codegraph status`
  Expected: all commands exit 0 and report tests; if Docker smoke cannot run, state it explicitly with the reason.
- [x] **Step 7: Deliver `git diff --stat` and key diff hunks**; do not commit.

## Self-Review

1. **Spec coverage:** per-connection view, scoped clear, live rows, local saves, old rows, mock/smoke/docs are all mapped to tasks.
2. **Step scan:** every step names one concrete action/signature/test; no TBD or generic “handle edge cases”.
3. **Type consistency:** `connectionId` option is consistently `Option<&str>` in Rust, `string`/`() => string` in TypeScript, and a string in JSON params.
4. **Review Focus:** all five edge classes have explicit tests in Task 1/2/3.
5. **Proportion:** plan describes contracts and tests, not full function bodies; each task is independently reviewable.
