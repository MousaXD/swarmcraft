export type StartupPhase = 'starting' | 'ready' | 'degraded' | 'failed';
export type StartupState = Readonly<{ phase: StartupPhase; detail: string }>;
export type StartupEvent =
  | { type: 'begin' }
  | { type: 'ready' }
  | { type: 'network-degraded'; detail: string }
  | { type: 'fail'; detail: string };

export const initialStartupState: StartupState = { phase: 'starting', detail: 'Setting up device' };

/** Presentation-only transitions. Never infer readiness from elapsed time. */
export function transitionStartup(current: StartupState, event: StartupEvent): StartupState {
  switch (event.type) {
    case 'begin': return initialStartupState;
    case 'fail': return { phase: 'failed', detail: event.detail };
    case 'ready':
      // A late successful callback cannot erase a failure or invent recovery.
      return current.phase === 'failed' ? current : { phase: 'ready', detail: 'Ready' };
    case 'network-degraded':
      return current.phase === 'failed'
        ? current
        : { phase: 'degraded', detail: event.detail };
  }
}
