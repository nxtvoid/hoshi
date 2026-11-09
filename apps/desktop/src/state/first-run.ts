/** Open the setup, save it as done without showing it, do nothing, or wait for more to load. */
export type FirstRunAction = 'welcome' | 'complete' | 'none' | 'wait'

export type FirstRunInput = {
  /** The snapshot arrived. */
  ready: boolean
  /** `undefined` while the settings aren't loaded. */
  completedAt: string | null | undefined
  hasProjects: boolean
  hasThreads: boolean
  /** Any event was ever stored, even if its projects are gone since. */
  hasHistory: boolean
  pathname: string
}

/** Only an untouched workspace gets the setup; anyone who used hoshi before skips it. */
export function firstRunAction({
  ready,
  completedAt,
  hasProjects,
  hasThreads,
  hasHistory,
  pathname
}: FirstRunInput): FirstRunAction {
  if (!ready || completedAt === undefined) return 'wait'
  if (completedAt !== null) return 'none'
  if (hasProjects || hasThreads || hasHistory) return 'complete'
  return pathname === '/' ? 'welcome' : 'wait'
}
