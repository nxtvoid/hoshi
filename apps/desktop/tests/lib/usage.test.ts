import { describe, expect, test } from 'bun:test'
import type { UsageWindow } from '../../src/bindings/UsageWindow'
import {
  contextPercent,
  formatTokens,
  paceOf,
  remainingPercent,
  timeLeftPercent,
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

  test('spending is compared to the time elapsed', () => {
    const halfway = '2026-11-08T00:30:00Z'

    expect(paceOf(session(80, halfway), NOW)).toBe('ahead')
    expect(paceOf(session(52, halfway), NOW)).toBe('on')
    expect(paceOf(session(10, halfway), NOW)).toBe('under')
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
