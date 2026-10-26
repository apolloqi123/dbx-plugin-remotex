/**
 * Mutable setup-scope primitives that several workbench wiring steps read and write.
 *
 * They are deliberately plain (non-reactive) values: grouping them in one object keeps
 * the read/write semantics identical to the single-scope App.vue setup they came from
 * (a `let` per value, shared by every closure defined after it). Reactive state lives in
 * `workbench/state/*`; this file only carries the imperative bookkeeping.
 *
 * One instance is created per workbench instance (see `useWorkbench`), so two mounted
 * workbenches never share these flags.
 */
export interface SetupState {
  /** Late-bound sidecar preference sync (assigned once prefs hydration is wired). */
  syncPrefsImpl: (() => Promise<void>) | undefined;
  /** Latest SetUserVar cwd report; null means the channel is not in use. */
  userVarCwdEvent: { path: string; at: number } | null;
  disposed: boolean;
  lastSequence: number;
  replayInFlight: boolean;
  replayNoProgress: number;
  pendingTerminalInput: string;
  activeTerminalSessionId: string;
  /** Terminal diagnostics interval handle (cleared on unmount). */
  terminalDiagTimer: number;
  /** Late-bound prompt-hint dismissal (the composable is created at the end of setup). */
  dismissPromptHintsImpl: (() => void) | undefined;
}

export function createSetupState(): SetupState {
  return {
    syncPrefsImpl: undefined,
    userVarCwdEvent: null,
    disposed: false,
    lastSequence: 0,
    replayInFlight: false,
    replayNoProgress: 0,
    pendingTerminalInput: "",
    activeTerminalSessionId: "",
    terminalDiagTimer: 0,
    dismissPromptHintsImpl: undefined,
  };
}
