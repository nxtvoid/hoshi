import { describe, expect, test } from 'bun:test'
import { withoutKeys } from '../../src/lib/records'

describe('withoutKeys', () => {
  test('drops the keys it is given', () => {
    const record = { a: 1, b: 2, c: 3 }
    expect(withoutKeys(record, ['a', 'c'])).toEqual({ b: 2 })
    expect(record).toEqual({ a: 1, b: 2, c: 3 })
  })

  test('returns the same record when none of the keys are there', () => {
    const record = { a: 1 }
    expect(withoutKeys(record, ['x'])).toBe(record)
    expect(withoutKeys(record, [])).toBe(record)
  })
})
