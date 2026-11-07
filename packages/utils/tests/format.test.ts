import { describe, expect, test } from 'bun:test'
import { relativeTimeParts } from '../src/format'

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
