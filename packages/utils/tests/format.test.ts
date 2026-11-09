import { describe, expect, test } from 'bun:test'
import {
  formatClockTime,
  formatDuration,
  formatElapsed,
  formatFileSize,
  relativeTimeParts
} from '../src/format'

const now = new Date('2026-11-07T12:00:00Z')
const ago = (seconds: number) => new Date(now.getTime() - seconds * 1000)

describe('relativeTimeParts', () => {
  test.each([
    [30, { value: 0, unit: 'second' }],
    [59, { value: 0, unit: 'second' }],
    [60, { value: -1, unit: 'minute' }],
    [90 * 60, { value: -1, unit: 'hour' }],
    [26 * 3_600, { value: -1, unit: 'day' }],
    [9 * 86_400, { value: -1, unit: 'week' }],
    [400 * 86_400, { value: -1, unit: 'year' }]
  ] as const)('%i seconds ago', (seconds, expected) => {
    expect(relativeTimeParts(ago(seconds), now)).toEqual(expected)
  })
})

describe('formatDuration', () => {
  test('shows the two largest units', () => {
    expect(formatDuration(72_000, 'en')).toBe('1m 12s')
    expect(formatDuration(3_725_000, 'en')).toBe('1h 2m')
    expect(formatDuration(400, 'en')).toBe('0s')
    expect(formatDuration(120_000, 'en')).toBe('2m')
  })
})

describe('formatElapsed', () => {
  test('reads like a clock, with hours only when it ran that long', () => {
    expect(formatElapsed(7_400)).toBe('0:07')
    expect(formatElapsed(750_000)).toBe('12:30')
    expect(formatElapsed(3_725_000)).toBe('1:02:05')
    expect(formatElapsed(-500)).toBe('0:00')
  })
})

describe('formatClockTime', () => {
  test('shows the hour and minute in the locale', () => {
    expect(formatClockTime('2026-11-08T22:10:00Z', 'es', 'UTC')).toBe('22:10')
    expect(formatClockTime('2026-11-08T22:10:00Z', 'en-US', 'UTC')).toBe(
      '10:10 PM'
    )
    expect(formatClockTime('not a date', 'es')).toBe('')
  })
})

describe('formatFileSize', () => {
  test('picks the largest unit that fits', () => {
    expect(formatFileSize(512, 'en')).toBe('512 byte')
    expect(formatFileSize(8_200, 'en')).toBe('8 kB')
    expect(formatFileSize(1_430_000, 'en')).toBe('1.4 MB')
  })
})
