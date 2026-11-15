# Terminal Recording UX and Pointer/Caret Capture Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Beautify the terminal recording panel, improve its controls, and record terminal-relative pointer/caret states so replay can visualize them without corrupting terminal output, search, transcript, or GIF export.

**Architecture:** Keep asciicast v2 `.cast` JSONL and the existing Rust recorder. Add a bounded custom `x` event with normalized pointer or terminal caret-cell payloads, ingested through SSH/local recording-event RPCs. Frontend captures bounded pointer/caret samples only while recording; replay parses `x` events into a separate track and renders an overlay in `.replay-terminal-wrap`. Output `o` events keep driving xterm unchanged.

**Tech Stack:** Vue 3 + TypeScript + xterm.js, Rust sidecar JSON-RPC + serde_json, Vitest/happy-dom, Rust `cargo test`, project mock host + registry validator, Python smoke test.

**Spec:** `docs/superpowers/specs/2026-11-15-recording-panel-design.md`

## Global Constraints

- No new framework, dependency, storage format, file extension, or MCP tool method.
- Record terminal-relative pointer coordinates and xterm caret cell only; never OS-global pointer coordinates, keystroke text, clipboard text, or file paths.
- Keep recordings directory 0700 and files 0600 on Unix; structured `x` payload cap is 512 bytes.
- `x` events never `term.write`; search, transcript, GIF, and existing `o` replay must ignore them.
- Legacy `.cast` files with only `o`/`m` events remain listable, searchable, replayable, and exportable.
- Preserve `PAGE_LIMIT = 500`, `REPLAY_EVENT_CAP = 20000`, `MAX_EVENT_BYTES = 256 * 1024`, `RECORDING_SEED_BYTES = 256 * 1024`.
- Add i18n keys consistently across all seven locales (`en`, `zh-CN`, `zh-TW`, `ja`, `es`, `it`, `pt-BR`) and preserve key/placeholder alignment.
- All frontend backend calls use literal method strings; every new backend method needs a mock implementation or a registry-check skip with a reason.
- Visible keyboard focus, 28px minimum action targets, Esc close, responsive down to mobile, and `prefers-reduced-motion` are mandatory.
- Do not commit, push, or package.
- After code edits: run `codegraph sync`, then `codegraph status`.

## Review Focus

1. **Malformed/oversized `x` payload:** backend rejects it and writes no event; Task 1 pins validation.
2. **No active recorder / wrong session kind:** event RPC returns the existing “no active recording” error and does not create a file; Task 1 pins this.
3. **Replay seek backward with metadata:** overlay shows the latest sample at/before playhead, not stale future state; Task 3 pins this.
4. **GIF/transcript with `x` events:** exported artifacts are identical to output-only replay; Task 3 pins this.
5. **Recording target disappears / host rebinding:** listeners and timers are disposed once, no event RPC fires after stop; Task 2 pins this.

---

## File Structure

- `backend/src/features/recording.rs`: `MAX_METADATA_EVENT_BYTES` and shared validation helper.
- `backend/src/features/recording/recorder.rs`: `observe_metadata` writer.
- `backend/src/features/recording/search.rs`: `read_events` typed `data` and output-only search.
- `backend/src/app/rpc/recording.rs`, `backend/src/app/rpc/local.rs`, `backend/src/sessions/ssh/recording.rs`, `backend/src/sessions/local/runtime/io.rs`: event ingress.
- `frontend/src/features/recording/lib/replayScheduler.ts`: typed pages, output/track splitting.
- `frontend/src/features/recording/lib/replayMotion.ts`: pure sample-at-time lookup.
- `frontend/src/features/recording/composables/useRecordingCapture.ts`: terminal pointer/caret capture + throttling.
- `frontend/src/features/recording/composables/useRecording.ts`, `useReplayPlayer.ts`: lifecycle and overlay state.
- `frontend/src/features/recording/components/RecordingsPanel.vue`, `frontend/src/app/components/TransferOverlays.vue`, `WorkbenchTerminalStage.vue`: panel redesign and replay overlay.
- `frontend/src/shared/lib/i18n/messages/supplemental/recordingMenu.ts`, `common.ts`: seven-locale keys.
- Tests, mock host, smoke, and protocol docs as listed in each task.

### Task 1: Backend bounded `x` event contract

**Files:**
- Modify: `backend/src/features/recording.rs`, `backend/src/features/recording/recorder.rs`, `backend/src/features/recording/search.rs`, `backend/src/app/rpc/recording.rs`, `backend/src/app/rpc/local.rs`, `backend/src/sessions/ssh/recording.rs`, `backend/src/sessions/local/runtime/io.rs`, `docs/architecture/PROTOCOL.zh-CN.md`
- Test: `backend/src/features/recording/tests.rs`, `scripts/smoke/smoke_fs_test.py`

**Interfaces:**
- Consumes: `SessionRecorder` with its monotonic `started_at` clock.
- Produces:
  - `pub const MAX_METADATA_EVENT_BYTES: usize = 512`
  - `pub fn validate_metadata_event(data: &Value) -> Result<Value, String>`
  - `pub fn SessionRecorder::observe_metadata(&mut self, data: &Value) -> Result<(), String>`
  - `pub async fn SshRuntime::recording_event(&self, session_id: &str, data: &Value) -> Result<Value, String>`
  - `pub async fn LocalTerminalRuntime::recording_event(&self, session_id: &str, data: &Value) -> Result<Value, String>`
  - RPCs `ssh/recording/event` and `local/recording/event`, both returning `{ "success": true }`.

- [x] **Step 1: Write failing Rust tests**: valid `{"kind":"pointer","x":0.4,"y":0.6,"pressed":false,"cols":100,"rows":26}` and `{"kind":"cursor","col":4,"row":9,"cols":100,"rows":26}` produce `eventtype: "x"`; oversized payload, NaN-like string, out-of-range coordinate, missing geometry, and unknown kind return errors without incrementing `events`; search for a token that only appears inside a structured `x` event returns no content hit; `read_events` returns the object intact for `x`.
- [x] **Step 2: Run `cargo test --manifest-path backend/Cargo.toml recording -- --nocapture`**. Expected: FAIL for missing symbols/contract.
- [x] **Step 3: Implement `validate_metadata_event`** in `backend/src/features/recording.rs`: require one of the two exact shapes, finite numeric values, `x/y ∈ [0,1]`, `pressed` boolean for pointer, `col ∈ [0,cols)`, `row ∈ [0,rows)`, integral values in `1..=1000` for `cols/rows`; reject unknown keys; return a compact canonical `Value`; reject serialized length above `MAX_METADATA_EVENT_BYTES`. Legacy event lines without `eventtype` still count as output.
- [x] **Step 4: Implement `SessionRecorder::observe_metadata`**: serialize canonical payload with `eventtype: "x"`, `time: started_at.elapsed().as_secs_f64()`, `eventdata: <object>`; increment `events`, add serialized bytes to `bytes`; write errors are swallowed like `observe`. Keep `finish`'s `m` marker unchanged.
- [x] **Step 5: Add event ingress methods**: `recording_event` locks the session recorder slot, validates first, errors with `"This session has no active recording"` if absent, then calls `observe_metadata`; expose the SSH method through `handle_recording`, and the local method through `handle_local`. Parsed `data` must be `params.get("data").cloned().unwrap_or(json!({}))`; require `sessionId`.
- [x] **Step 6: Update search and read path**: `flatten_event_text` returns `""` for non-output events while retaining output semantics for legacy lines; `read_events` keeps `data` as the original JSON `Value` (not coerced to string). Add a test that a page with one `o` and one `x` returns both with correct `type`.
- [x] **Step 7: Update smoke and protocol**: in both SSH/local recording flows, send one pointer and one cursor `x` event, assert `ssh/recording/get` includes both `type: "x"` rows and the output marker is still present; document both new RPCs, `x` event shape, 512-byte cap, output-only search/export semantics, and legacy compatibility in `docs/architecture/PROTOCOL.zh-CN.md`. Add both methods to `SKIP_EXACT` in `scripts/verify/mock_host_registry_check.mjs` if the mock host does not implement them, with a comment; prefer implementing no-op mock branches instead.
- [x] **Step 8: Run `cargo test --manifest-path backend/Cargo.toml recording -- --nocapture`** and `node scripts/verify/mock_host_registry_check.mjs`. Expected: PASS; report Docker smoke as skipped if no fixture is available.

### Task 2: Frontend capture and event typing

**Files:**
- Create: `frontend/src/features/recording/lib/replayScheduler.ts` (extend), `frontend/src/features/recording/lib/replayMotion.ts`, `frontend/src/features/recording/composables/useRecordingCapture.ts`, `frontend/src/features/recording/lib/replayMotion.spec.ts`
- Modify: `frontend/src/features/recording/composables/useRecording.ts`, `frontend/src/app/workbench/setup/step8-shell.ts`, `frontend/src/app/mock/host/transfers.ts`, `frontend/src/features/recording/lib/replayScheduler.spec.ts`
- Test: `frontend/src/features/recording/lib/replayScheduler.spec.ts`, `replayMotion.spec.ts`, `frontend/src/app/mockDbxHost.spec.ts`

**Interfaces:**
- Consumes: Task 1's `ssh/recording/event` / `local/recording/event` and canonical `x` payloads.
- Produces:
  - `interface ReplayEventPage { events: Array<{ time: number; type: string; data: string | RecordingMotionData }>; ... }`
  - `type RecordingMotionData = PointerMotion | CursorMotion`
  - `function splitReplayPages(pages: readonly ReplayEventPage[]): { events: ReplayEvent[]; motion: ReplayMotionEvent[] }`
  - `interface ReplayMotionEvent { time: number; data: RecordingMotionData }`
  - `function motionAtTime(motion: readonly ReplayMotionEvent[], timeMs: number): { pointer: PointerMotion | null; cursor: CursorMotion | null }`
  - `function useRecordingCapture(options: { recordingActive: Ref<boolean>; target: Ref<{kind:"ssh"|"local";sessionId:string}|undefined>; terminalHost: Ref<HTMLElement|undefined>; terminal: () => Terminal | undefined }): { dispose(): void }`

- [x] **Step 1: Write failing pure tests**: `splitReplayPages` drops `x` from terminal events, preserves `o`/`i`/`m` behavior; `motionAtTime` returns the last pointer and last cursor independently at/before a playhead and `null` before the first sample. `mergeEventPages` remains output-only and now narrows `data` to strings.
- [x] **Step 2: Run `cd frontend && pnpm vitest run src/features/recording/lib/replayScheduler.spec.ts src/features/recording/lib/replayMotion.spec.ts`**. Expected: FAIL for missing `splitReplayPages`/`motionAtTime`.
- [x] **Step 3: Extend `replayScheduler.ts`**: widen page `data` to `string | RecordingMotionData`; keep `mergeEventPages` exactly output-only; add `splitReplayPages` that gathers only valid `type === "x"` objects into `motion` and all output events into `events`. Unknown/malformed `x` objects are ignored. Export the motion types from their own pure module.
- [x] **Step 4: Implement `motionAtTime`** in `replayMotion.ts`: one linear scan, independent latest pointer/cursor at or before `timeMs / 1000`, with a stable lower-bound fallback for negative time; no Vue/DOM dependencies.
- [x] **Step 5: Implement `useRecordingCapture`**: attach `pointermove`, `pointerdown`, and `pointerup` to the current `terminalHost` via a `watch(..., { flush: "post" })`; throttle pointer samples to at most 20/s, and caret samples to at most 10/s from `terminal()?.buffer.active.cursorX/cursorY`; normalize host-relative `x/y` into 0..1, clamp, include current `cols/rows`; send `rpc("ssh/recording/event", ...)` or `rpc("local/recording/event", ...)` fire-and-forget with `.catch(() => undefined)`; stop all work when `recordingActive` is false or target is undefined; `dispose()` removes listeners/timers once. Do not capture text or OS coordinates.
- [x] **Step 6: Wire `useRecording`**: add optional `terminalHost` and `terminal` getters to options, instantiate capture after `recordingTarget` is available, call `dispose()` in `onBeforeUnmount`; add `refreshRecordings` action alias for `loadRecordings` if the panel needs a distinct emitted event. Pass `terminalHost` and `terminal: () => terminal.value` from `step8-shell.ts` (`terminal`/`terminalHost` are already in its deps).
- [x] **Step 7: Update mock host**: add no-op `ssh/recording/event` and `local/recording/event` branches returning `{ success: true }`; add two `x` fixture events to `RECORDING_FIXTURE_EVENTS` (one pointer, one cursor) and keep content-search filtering only string `o.data`; add a mock host test asserting `ssh/recording/get` includes the new event type but search for `"pointer"` does not produce a content hit.
- [x] **Step 8: Run `cd frontend && pnpm typecheck && pnpm vitest run src/features/recording/lib src/app/mockDbxHost.spec.ts`**. Expected: PASS.

### Task 3: Replay overlay and export safety

**Files:**
- Modify: `frontend/src/features/recording/composables/useRecording.ts`, `frontend/src/features/recording/composables/useReplayPlayer.ts`, `frontend/src/app/components/TransferOverlays.vue`, `frontend/src/app/components/WorkbenchTerminalStage.vue`, `frontend/src/features/recording/lib/recordingExport.ts`, `frontend/src/features/recording/lib/transcript.ts`, tests for scheduler/transcript/export
- Test: `frontend/src/features/recording/lib/replayScheduler.spec.ts`, `transcript.spec.ts`, `frontend/src/features/recording/composables/useReplayPlayer.spec.ts` (create if needed)

**Interfaces:**
- Consumes: Task 2's `splitReplayPages` and `motionAtTime`.
- Produces:
  - `type ReplayLoadResult = { events: ReplayEvent[]; motion: ReplayMotionEvent[] }`
  - `useRecording.loadReplayEvents` returns `ReplayEvent[]`; `loadReplayRecording` returns `ReplayLoadResult`.
  - `useReplayPlayer.loadReplayRecording: (recordingId: string) => Promise<ReplayLoadResult>`
  - `replayState: Ref<{ summary; events; motion; pointer: PointerMotion | null; cursor: CursorMotion | null } | null>`

- [x] **Step 1: Write failing tests**: a page containing `o` and `x` yields an unchanged GIF frame plan and transcript string; seeking backward resets both overlay positions to prior samples; an empty/legacy recording yields `pointer: null, cursor: null` and no overlay text.
- [x] **Step 2: Run targeted Vitest**. Expected: FAIL for the missing load result/motion state.
- [x] **Step 3: Split loading in `useRecording.ts`**: keep `loadReplayEvents` for GIF/transcript (output-only); add `loadReplayRecording` that uses the same paginated RPC and returns `splitReplayPages`; pass it to `useReplayPlayer`. Explicitly ensure `recordingExport.ts` and `transcript.ts` still consume only `ReplayEvent[]`; update `TranscriptEvent` to tolerate object `data` but skip any `type !== "o"` row before reading text.
- [x] **Step 4: Extend `useReplayPlayer`**: store `motion`, compute `pointer`/`cursor` in `replayState` from `motionAtTime`, update them in `replayFrame`, `openReplay` at t=0, and `onReplaySeek`; reset them in `closeReplay`; keep writing only `state.events[*].data` to xterm. Use a pure helper for the state patch so the seek test can call it.
- [x] **Step 5: Add replay overlay** in `TransferOverlays.vue`: inside `.replay-terminal-wrap`, add `pointer-events:none` absolutely positioned pointer dot and caret outline; pointer position uses `left: x*100%`, `top: y*100%`; caret uses `col/cols`, `row/rows` percentages with a one-cell box; add a compact toggle button (`replayMotionToggle`) + a `replayMotionEnabled` prop/ref so users can hide the overlay; use `aria-hidden="true"` for decorative marks and a labeled button; pass props/events through `WorkbenchTerminalStage.vue`.
- [x] **Step 6: Add seven-locale i18n keys** in `recordingMenuMessages` and `commonMessages` as appropriate: `recordingsRefresh`, `recordingsStop`, `recordingsCapturingHint`, `recordingsKeyboardHint`, `replayMotionToggle`, `replayMotionOn`, `replayMotionOff`; keep exact key sets and placeholder alignment; use plain sentence-case labels, no ALL CAPS/eyebrows/arrow suffixes.
- [x] **Step 7: Run `cd frontend && pnpm typecheck && pnpm test`**. Expected: PASS with all seven-locale alignment tests.

### Task 4: Recording panel redesign and interactions

**Files:**
- Modify: `frontend/src/features/recording/components/RecordingsPanel.vue`, `frontend/src/app/components/WorkbenchTerminalStage.vue`, `frontend/src/app/workbench/setup/step8-shell.ts`, `frontend/src/app/styles/global/cards.css`, `frontend/src/features/recording/components/RecordingMenu.spec.ts` (or add `RecordingsPanel.spec.ts`)
- Test: `frontend/src/features/recording/components/RecordingsPanel.spec.ts`, `frontend/src/features/recording/components/RecordingMenu.spec.ts`, `frontend/src/app/workbench.spec.ts`

**Interfaces:**
- Consumes: `useRecording.loadRecordings`, `toggleRecording`, `cancelRecordCountdown`, `recordingActive`, `recordCountdown`, `recordingElapsedSec`, and existing row actions.
- Produces: `defineEmits` adds `"refresh"`, `"stop-recording"`, `"cancel-countdown"`, `"refresh-recording"`, and `"open-recording-row"`; existing emits remain.

- [x] **Step 1: Write failing component tests** with happy-dom + `attachTo: document.body`: recording state shows Stop; countdown state shows Cancel start; header has Refresh; Enter on a focused row calls `open-replay`; Esc calls `close`; action buttons have visible `aria-label`s; search-clear and refresh emit correctly.
- [x] **Step 2: Run `cd frontend && pnpm vitest run src/features/recording/components`**. Expected: FAIL for missing controls/events.
- [x] **Step 3: Redesign `RecordingsPanel.vue`** following the approved design: 520px scoped width, a single tape spine with state-colored nodes, host as row title, time/size/duration in tabular mono, visible 28px action targets (replay, GIF, transcript, reveal, delete), search hit excerpts, loading/empty/no-match states with explicit next action. Replace hover-only action opacity with persistent accessible controls. Header: status line, refresh, stop (or cancel countdown), clear-all, close. Footer: keyboard hint + start recording. Use `role="list"/"listitem"`, `tabindex="0"`, Enter opens replay, Escape closes; focus-visible rings must remain visible. Respect reduced motion via global token rules and avoid non-user-triggered animation.
- [x] **Step 4: Wire new events** through `WorkbenchTerminalStage.vue`: `@refresh="loadRecordings"`, `@stop-recording="toggleRecording"` when active, `@cancel-countdown="cancelRecordCountdown"`, `@refresh-recording="loadRecordings"`; keep clear/delete/reveal/export/replay wiring unchanged. `cancelRecordCountdown` is already returned from `useRecording`.
- [x] **Step 5: Reconcile global/local styles**: move duplicate `.recordings-float { width: min(440px, ...) }` from `cards.css` into the component’s single width declaration if safe, or delete the stale rule; ensure no selector specificity conflict. Keep existing `--card-*`, `--tone-*`, `--radius-*`, `--space-*`, and `--text-*` tokens.
- [x] **Step 6: Run `cd frontend && pnpm typecheck && pnpm test`**. Expected: PASS.

### Task 5: Full verification and review-ready diff

**Files:**
- All modified files; no new product files beyond those listed.

**Interfaces:**
- Consumes: Tasks 1–4.
- Produces: verification log and uncommitted diff for Claude review.

- [x] **Step 1: Run static/contract checks**:
  - `node scripts/verify/mock_host_registry_check.mjs`
  - `python3 -I scripts/verify/validate_repo.py`
  - `cd frontend && pnpm typecheck && pnpm test`
  - `cargo test --manifest-path backend/Cargo.toml`
  Expected: all exit 0; report exact frontend/backend test counts.
- [x] **Step 2: Run Docker smoke if available**: `python3 -I scripts/smoke/smoke_fs_test.py` only with the project's local SSH fixture; assert SSH/local `x` events and per-connection history. If the fixture is unavailable, report the exact skipped command/reason rather than claiming it passed.
- [x] **Step 3: Run `codegraph sync && codegraph status`**. Expected: index up to date; `.codegraph/` remains ignored.
- [x] **Step 4: Deliver `git diff --stat`, key diff hunks, changed-file list, and any unresolved warnings**. Do not commit, push, package, or close the Orca pane yourself.

## Self-Review

1. **Spec coverage:** UI visual/interaction redesign, pointer/caret capture, replay visualization, bounded events, legacy compatibility, export/search safety, i18n, and accessibility each map to a task.
2. **Step scan:** each step names a concrete symbol, test, or command; design implementation is intentionally left to the worker within pinned constraints.
3. **Type consistency:** `ReplayEvent.data` remains `string`; motion data lives in `ReplayMotionEvent.data`; RPC `data` is a canonical JSON object; all state names are consistent across tasks.
4. **Review Focus:** all five failure classes have explicit tests or smoke assertions.
5. **Proportion:** plan binds architectural decisions and contracts without transcribing component markup or function bodies.
