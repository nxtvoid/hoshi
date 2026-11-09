import { expect, test } from 'bun:test'
import type { ContextUsage } from '../../src/bindings/ContextUsage'
import { compactionDue, isCompactCommand } from '../../src/lib/compaction'

const usage = (
  usedTokens: number,
  cacheExpiresAt: number | null
): ContextUsage => ({
  usedTokens,
  maxTokens: 1_000_000,
  cacheTtlSecs: 3600,
  cacheExpiresAt
})

test('a large context whose cache ran out compacts first, unless a turn runs', () => {
  expect(compactionDue(usage(250_000, 1_000), 2_000, false)).toBe(true)
  expect(compactionDue(usage(250_000, 1_000), 2_000, true)).toBe(false)
})

test('a small context, a warm cache or an unknown one sends as is', () => {
  expect(compactionDue(usage(150_000, 1_000), 2_000, false)).toBe(false)
  expect(compactionDue(usage(250_000, 3_000), 2_000, false)).toBe(false)
  expect(compactionDue(usage(250_000, null), 2_000, false)).toBe(false)
  expect(compactionDue(undefined, 2_000, false)).toBe(false)
})

test('the user typing /compact is already a compaction', () => {
  expect(isCompactCommand(' /compact ')).toBe(true)
  expect(isCompactCommand('/compact keep the API notes')).toBe(true)
  expect(isCompactCommand('/compactor')).toBe(false)
  expect(isCompactCommand('please /compact')).toBe(false)
})
