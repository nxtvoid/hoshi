import { describe, expect, test } from 'bun:test'
import { textCache } from '../../src/lib/text-cache'

describe('textCache', () => {
  test('drops the least recently used once the texts outgrow the limit', () => {
    const cache = textCache<number>(10)
    cache.set('aaaa', 1)
    cache.set('bbbb', 2)
    expect(cache.get('aaaa')).toBe(1)
    cache.set('cccc', 3)

    expect(cache.get('bbbb')).toBeUndefined()
    expect(cache.get('aaaa')).toBe(1)
    expect(cache.get('cccc')).toBe(3)
  })

  test('never keeps a text longer than the limit', () => {
    const cache = textCache<number>(3)
    cache.set('abcd', 1)
    expect(cache.get('abcd')).toBeUndefined()
  })

  test('setting a text again counts it once', () => {
    const cache = textCache<number>(8)
    cache.set('aaaa', 1)
    cache.set('aaaa', 2)
    cache.set('bbbb', 3)

    expect(cache.get('aaaa')).toBe(2)
    expect(cache.get('bbbb')).toBe(3)
  })
})
