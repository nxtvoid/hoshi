import type { UsageLimits } from '@/bindings/UsageLimits'
import type { UsageWindow } from '@/bindings/UsageWindow'
import type { BankedResets } from '@/bindings/BankedResets'
import type { ContextUsage } from '@/bindings/ContextUsage'

export type Pace = 'ahead' | 'on' | 'under'

const MINUTE = 60_000
const HOUR = 60 * MINUTE
const DAY = 24 * HOUR
const PACE_TOLERANCE = 5

/** 0 to 100, or `null` while the window size is unknown. */
export function contextPercent(usage: ContextUsage): number | null {
  if (!usage.maxTokens) return null
  return Math.min(100, (usage.usedTokens / usage.maxTokens) * 100)
}

const tokenFormatters = new Map<string, Intl.NumberFormat>()

export function formatTokens(tokens: number, locale: string) {
  let formatter = tokenFormatters.get(locale)
  if (!formatter) {
    formatter = new Intl.NumberFormat(locale, {
      notation: 'compact',
      maximumFractionDigits: 1
    })
    tokenFormatters.set(locale, formatter)
  }
  return formatter.format(tokens)
}

export function remainingPercent(window: UsageWindow) {
  return 100 - Math.min(100, Math.max(0, window.usedPercent))
}

function millisUntil(time: string | null, now: number): number | null {
  if (!time) return null
  const at = Date.parse(time)
  return Number.isFinite(at) ? Math.max(0, at - now) : null
}

function millisToReset(window: UsageWindow, now: number): number | null {
  return millisUntil(window.resetsAt, now)
}

/** How much of the window's time is left, 0 to 100: where even spending would leave the quota. */
export function timeLeftPercent(window: UsageWindow, now: number) {
  const left = millisToReset(window, now)
  const length = window.windowMinutes * MINUTE
  if (left === null || length <= 0) return null
  return Math.min(100, (left / length) * 100)
}

export function paceOf(window: UsageWindow, now: number): Pace | null {
  const timeLeft = timeLeftPercent(window, now)
  if (timeLeft === null) return null
  const gap = window.usedPercent - (100 - timeLeft)
  if (gap > PACE_TOLERANCE) return 'ahead'
  if (gap < -PACE_TOLERANCE) return 'under'
  return 'on'
}

export type Duration = { days: number; hours: number; minutes: number }

function durationOf(millis: number): Duration {
  return {
    days: Math.floor(millis / DAY),
    hours: Math.floor((millis % DAY) / HOUR),
    minutes: Math.floor((millis % HOUR) / MINUTE)
  }
}

export function timeToReset(window: UsageWindow, now: number): Duration | null {
  const left = millisToReset(window, now)
  return left === null ? null : durationOf(left)
}

/** The banked resets that can be spent now; `null` when there are none to show. */
export function spendableResets(
  limits: UsageLimits | null
): BankedResets | null {
  const banked = limits?.bankedResets
  return banked && banked.count > 0 && banked.nextGrantId ? banked : null
}

/** How long until the soonest banked reset expires; `null` when none does. */
export function timeToExpiry(
  banked: BankedResets,
  now: number
): Duration | null {
  const left = banked.expiresAt
    .map((time) => millisUntil(time, now))
    .find((millis) => millis !== null && millis > 0)
  return left == null ? null : durationOf(left)
}
