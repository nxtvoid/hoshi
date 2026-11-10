import type { UsageLimits } from '@/bindings/UsageLimits'
import type { UsageWindow } from '@/bindings/UsageWindow'
import type { BankedResets } from '@/bindings/BankedResets'
import type { ContextUsage } from '@/bindings/ContextUsage'

const MINUTE = 60_000
const HOUR = 60 * MINUTE
const DAY = 24 * HOUR

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

/** The banked resets a window's row shows: only the session window covering every model has them. */
export function bankedResetsFor(
  window: UsageWindow,
  limits: UsageLimits | null
): BankedResets | null {
  return window.kind === 'session' && window.model === null
    ? spendableResets(limits)
    : null
}

/** Where even spending would leave the quota by now, rounded; `null` while the reset is unknown. */
export function evenPaceLeft(window: UsageWindow, now: number) {
  const left = timeLeftPercent(window, now)
  return left === null ? null : Math.round(left)
}

/** The share of the quota the reset gives back. */
export function restoredPercent(window: UsageWindow) {
  return 100 - remainingPercent(window)
}

export type ResetDay = 'today' | 'tomorrow' | 'other'

const dateFormatters = new Map<string, Intl.DateTimeFormat>()

function dateFormatter(locale: string, kind: 'time' | 'weekday' | 'date') {
  const key = `${locale}:${kind}`
  let formatter = dateFormatters.get(key)
  if (!formatter) {
    const time = { hour: 'numeric', minute: '2-digit' } as const
    formatter = new Intl.DateTimeFormat(locale, {
      ...time,
      ...(kind === 'weekday' && { weekday: 'short' }),
      ...(kind === 'date' && { month: 'short', day: 'numeric' })
    })
    dateFormatters.set(key, formatter)
  }
  return formatter
}

function startOfDay(time: number) {
  const date = new Date(time)
  date.setHours(0, 0, 0, 0)
  return date.getTime()
}

/**
 * When a window resets in local time: the time alone today or tomorrow, the weekday within
 * a week, the date after. `null` while the reset is unknown.
 */
export function resetMoment(
  window: UsageWindow,
  now: number,
  locale: string
): { day: ResetDay; text: string } | null {
  if (!window.resetsAt) return null
  const at = Date.parse(window.resetsAt)
  if (!Number.isFinite(at)) return null
  const days = Math.round((startOfDay(at) - startOfDay(now)) / DAY)
  if (days <= 0)
    return { day: 'today', text: dateFormatter(locale, 'time').format(at) }
  if (days === 1) {
    return { day: 'tomorrow', text: dateFormatter(locale, 'time').format(at) }
  }
  const kind = days < 7 ? 'weekday' : 'date'
  return { day: 'other', text: dateFormatter(locale, kind).format(at) }
}
