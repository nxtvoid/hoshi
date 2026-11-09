import type { ProviderUpdate } from '@/bindings/ProviderUpdate'
import type { ProviderVersion } from '@/bindings/ProviderVersion'

/** An update the sidebar's button points at; `null` when there is none. */
export type UpdateBadge =
  | 'available'
  | 'running'
  | 'ready'
  | 'failed'
  | 'done'
  | null

/** hoshi's own update, from its release feed. */
export type AppUpdate =
  | { status: 'idle' }
  | { status: 'checking' }
  | { status: 'upToDate'; version: string }
  | { status: 'checkFailed'; message: string }
  | { status: 'available'; version: string }
  | { status: 'downloading'; version: string; percent: number | null }
  | { status: 'ready'; version: string }
  | { status: 'failed'; version: string }

/** What the sidebar's update button does: check the feed, or open the update details. */
export type UpdateButton =
  | { kind: 'off' }
  | { kind: 'check'; checking: boolean }
  | { kind: 'upToDate'; version: string }
  | { kind: 'checkFailed'; message: string }
  | { kind: 'details'; badge: NonNullable<UpdateBadge> }

export function updateButton(
  app: AppUpdate,
  provider: UpdateBadge,
  enabled: boolean
): UpdateButton {
  const badge = strongestBadge(appBadge(app), provider)
  if (badge) return { kind: 'details', badge }
  if (!enabled) return { kind: 'off' }
  switch (app.status) {
    case 'checking':
      return { kind: 'check', checking: true }
    case 'upToDate':
      return { kind: 'upToDate', version: app.version }
    case 'checkFailed':
      return { kind: 'checkFailed', message: app.message }
    default:
      return { kind: 'check', checking: false }
  }
}

/** A check is due on focus once the last one is this old. */
export const RECHECK_ON_FOCUS_AFTER = 2 * 60 * 1000

export function checkDue(lastCheckedAt: number | null, now: number) {
  return lastCheckedAt === null || now - lastCheckedAt >= RECHECK_ON_FOCUS_AFTER
}

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
    case 'available':
      return 'available'
    case 'downloading':
      return 'running'
    case 'ready':
      return 'ready'
    case 'failed':
      return 'failed'
    default:
      return null
  }
}

const BADGE_WEIGHT: Record<NonNullable<UpdateBadge>, number> = {
  running: 5,
  ready: 4,
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

/** Caches a release range's notes for the session. */
export function releaseNotesKey(installed: string, available: string) {
  return `${installed}...${available}`
}

/** A release body's bullet points, as plain text. */
export function releaseChanges(body: string) {
  return body
    .split(/\r?\n/)
    .map((line) => /^\s*[-*+]\s+(.+)$/.exec(line)?.[1]?.trim())
    .filter((change): change is string => Boolean(change))
}

export function releaseCompareUrl(installed: string, available: string) {
  return `https://github.com/nxtvoid/hoshi/compare/v${installed}...v${available}`
}

/** The version whose release notes the update details show; `null` when they show none. */
export function releaseNotesTarget(update: AppUpdate) {
  return update.status === 'available' ||
    update.status === 'downloading' ||
    update.status === 'ready'
    ? update.version
    : null
}
