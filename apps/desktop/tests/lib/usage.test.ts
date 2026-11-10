import { describe, expect, test } from 'bun:test'
import type { UsageLimits } from '../../src/bindings/UsageLimits'
import type { UsageWindow } from '../../src/bindings/UsageWindow'
import {
  bankedResetsFor,
  contextPercent,
  evenPaceLeft,
  formatTokens,
  remainingPercent,
  resetMoment,
  restoredPercent,
  spendableResets,
  timeLeftPercent,
  timeToExpiry,
  timeToReset
} from '../../src/lib/usage'

const NOW = Date.parse('2026-11-07T22:00:00Z')

const session = (
  usedPercent: number,
  resetsAt: string | null
): UsageWindow => ({
  kind: 'session',
  model: null,
  usedPercent,
  windowMinutes: 300,
  resetsAt
})

describe('context', () => {
  test('a percentage needs the window size', () => {
    expect(contextPercent({ usedTokens: 50_000, maxTokens: 200_000 })).toBe(25)
    expect(contextPercent({ usedTokens: 50_000, maxTokens: null })).toBeNull()
  })

  test('tokens read in compact form', () => {
    expect(formatTokens(84_000, 'en')).toBe('84K')
    expect(formatTokens(1_000_000, 'en')).toBe('1M')
    expect(formatTokens(950, 'en')).toBe('950')
  })
})

describe('limits', () => {
  test('the bar shows what is left', () => {
    expect(remainingPercent(session(33, null))).toBe(67)
  })

  test('the pace mark sits at the share of time left', () => {
    const window = session(0, '2026-11-08T00:30:00Z')

    expect(timeLeftPercent(window, NOW)).toBe(50)
    expect(timeLeftPercent(session(0, null), NOW)).toBeNull()
  })

  test('the time to reset is split into days, hours and minutes', () => {
    const window = {
      ...session(0, '2026-11-12T13:17:00Z'),
      windowMinutes: 10_080
    }

    expect(timeToReset(window, NOW)).toEqual({
      days: 4,
      hours: 15,
      minutes: 17
    })
    expect(timeToReset(session(0, '2026-11-07T21:00:00Z'), NOW)).toEqual({
      days: 0,
      hours: 0,
      minutes: 0
    })
  })
})

describe('banked resets', () => {
  const limits = (bankedResets: UsageLimits['bankedResets']): UsageLimits => ({
    windows: [session(40, null)],
    bankedResets,
    account: null
  })
  const banked = {
    count: 2,
    nextGrantId: 'grant_a',
    expiresAt: ['2026-11-07T21:00:00Z', '2027-03-07T04:30:00Z']
  }

  test('show only while one can be spent', () => {
    expect(spendableResets(limits(banked))).toBe(banked)
    expect(spendableResets(limits({ ...banked, count: 0 }))).toBeNull()
    expect(spendableResets(limits({ ...banked, nextGrantId: null }))).toBeNull()
    expect(spendableResets(limits(null))).toBeNull()
    expect(spendableResets(null)).toBeNull()
  })

  test('expire with the soonest grant still to come', () => {
    expect(timeToExpiry(banked, NOW)).toEqual({
      days: 119,
      hours: 6,
      minutes: 30
    })
    expect(timeToExpiry({ ...banked, expiresAt: [] }, NOW)).toBeNull()
  })
})

describe('window detail', () => {
  const local = (day: number, hour: number, minute = 0) =>
    new Date(2026, 10, day, hour, minute).getTime()
  const resetAt = (time: number) => session(26, new Date(time).toISOString())
  const time = (at: number) =>
    new Intl.DateTimeFormat('en', {
      hour: 'numeric',
      minute: '2-digit'
    }).format(at)
  const now = local(9, 10, 47)

  test('a reset reads as today, tomorrow, a weekday or a date', () => {
    expect(resetMoment(resetAt(local(9, 14, 10)), now, 'en')).toEqual({
      day: 'today',
      text: time(local(9, 14, 10))
    })
    expect(resetMoment(resetAt(local(10, 2, 10)), now, 'en')).toEqual({
      day: 'tomorrow',
      text: time(local(10, 2, 10))
    })
    const thursday = resetMoment(resetAt(local(12, 9)), now, 'en')
    expect(thursday?.day).toBe('other')
    expect(thursday?.text).toMatch(/^Thu.*9:00\sAM$/)
    expect(resetMoment(resetAt(local(20, 9)), now, 'en')?.text).toMatch(
      /^Nov 20.*9:00\sAM$/
    )
    expect(resetMoment(session(26, null), now, 'en')).toBeNull()
  })

  test("the 12-hour clock is the locale's choice", () => {
    expect(resetMoment(resetAt(local(9, 14, 10)), now, 'de')?.text).toBe(
      '14:10'
    )
  })

  test('even pace is the share of time left, rounded', () => {
    const window = session(0, '2026-11-08T01:24:00Z')

    expect(evenPaceLeft(window, NOW)).toBe(68)
    expect(evenPaceLeft(session(0, null), NOW)).toBeNull()
  })

  test('the reset gives back what was used', () => {
    expect(restoredPercent(session(26, null))).toBe(26)
    expect(restoredPercent(session(120, null))).toBe(100)
  })

  test('only the session row covering every model shows banked resets', () => {
    const banked = { count: 1, nextGrantId: 'grant_a', expiresAt: [] }
    const weekly: UsageWindow = { ...session(10, null), kind: 'weekly' }
    const scoped: UsageWindow = { ...session(10, null), model: 'Fable' }
    const limits: UsageLimits = {
      windows: [session(10, null), weekly, scoped],
      bankedResets: banked,
      account: null
    }

    expect(bankedResetsFor(session(10, null), limits)).toBe(banked)
    expect(bankedResetsFor(weekly, limits)).toBeNull()
    expect(bankedResetsFor(scoped, limits)).toBeNull()
    expect(
      bankedResetsFor(session(10, null), {
        ...limits,
        bankedResets: { ...banked, count: 0 }
      })
    ).toBeNull()
  })
})
