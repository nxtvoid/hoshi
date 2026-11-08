import type { ProjectSignal } from '@/state/thread-signals'

/** A rail button's own border tells what its threads are doing. */
export const SIGNAL_BORDER: Record<ProjectSignal, string> = {
  error: 'border-status-error [--signal:var(--status-error)]',
  waiting: 'border-status-waiting [--signal:var(--status-waiting)]',
  done: 'border-status-done [--signal:var(--status-done)]',
  working: 'border-status-working [--signal:var(--status-working)]'
}
