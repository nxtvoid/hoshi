import { expect, test } from 'bun:test'
import { offsets } from '../src/offsets'

test('each size starts where the ones before it end', () => {
  expect(offsets([2, 3, 1])).toEqual([0, 2, 5])
  expect(offsets([])).toEqual([])
})
