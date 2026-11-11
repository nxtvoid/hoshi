import { expect, test } from 'bun:test'
import { withCleanup } from '../src/cleanup'

test('cleans up after work that succeeds or fails, and keeps its outcome', async () => {
  const done: string[] = []
  expect(
    await withCleanup(
      async () => 1,
      () => done.push('ok')
    )
  ).toBe(1)
  await expect(
    withCleanup(
      async () => {
        throw new Error('no')
      },
      () => done.push('failed')
    )
  ).rejects.toThrow('no')
  expect(done).toEqual(['ok', 'failed'])
})
