import type { ProviderUpdate } from '@/bindings/ProviderUpdate'
import type { ProviderVersion } from '@/bindings/ProviderVersion'

/** What the sidebar's update button shows; `null` hides it. */
export type UpdateBadge = 'available' | 'running' | 'failed' | 'done' | null

/** hoshi's own update, from its release feed. */
export type AppUpdate =
  | { status: 'idle' }
  | { status: 'available'; version: string }
  | { status: 'downloading'; version: string; percent: number | null }
  | { status: 'failed'; version: string }

/** `seen`: the user already looked at how the last provider update ended. */
export function providerBadge(
  version: ProviderVersion | null,
  seen: boolean
): UpdateBadge {
  if (!version) return null
  const { status } = version.update
  if (status === 'running') return 'running'
  if (!seen && (status === 'failed' || status === 'unchanged')) return 'failed'
  if (!seen && status === 'succeeded') return 'done'
  return version.updateAvailable ? 'available' : null
}

export function appBadge(update: AppUpdate): UpdateBadge {
  switch (update.status) {
    case 'idle':
      return null
    case 'available':
      return 'available'
    case 'downloading':
      return 'running'
    case 'failed':
      return 'failed'
  }
}

const BADGE_WEIGHT: Record<NonNullable<UpdateBadge>, number> = {
  running: 4,
  failed: 3,
  done: 2,
  available: 1
}

/** The badge that says the most, so a running update is never hidden. */
export function strongestBadge(...badges: UpdateBadge[]): UpdateBadge {
  return badges.reduce<UpdateBadge>(
    (strongest, badge) =>
      badge && (!strongest || BADGE_WEIGHT[badge] > BADGE_WEIGHT[strongest])
        ? badge
        : strongest,
    null
  )
}

/** An update ended since `previous`, so its outcome is worth showing again. */
export function updateEnded(
  previous: ProviderUpdate | undefined,
  next: ProviderUpdate
) {
  return previous?.status === 'running' && next.status !== 'running'
}

/** Whole percent of a download; `null` while its size is unknown. */
export function downloadPercent(received: number, total: number) {
  if (total <= 0) return null
  return Math.min(100, Math.floor((received / total) * 100))
}
