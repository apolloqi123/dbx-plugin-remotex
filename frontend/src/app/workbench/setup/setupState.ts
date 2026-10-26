/**
 * Mutable setup-scope primitives that several workbench wiring steps read and write.
 *
 * They are deliberately plain (non-reactive) values: grouping them in one object keeps
 * the read/write semantics identical to the single-scope App.vue setup they came from
 * (a `let` per value, shared by every closure defined after it). Reactive state lives in
 * `workbench/state/*`; this file only carries the imperative bookkeeping.
 */
export const setupState = {
  /** Late-bound sidecar preference sync (assigned once prefs hydration is wired). */
  syncPrefsImpl: undefined as (() => Promise<void>) | undefined,
  /** Latest SetUserVar cwd report; null means the channel is not in use. */
  userVarCwdEvent: null as { path: string; at: number } | null,
  disposed: false,
  lastSequence: 0,
  replayInFlight: false,
  replayNoProgress: 0,
  pendingTerminalInput: "",
  activeTerminalSessionId: "",
  /** Terminal diagnostics interval handle (cleared on unmount). */
  terminalDiagTimer: 0,
  /** Late-bound prompt-hint dismissal (the composable is created at the end of setup). */
  dismissPromptHintsImpl: undefined as (() => void) | undefined,
};
