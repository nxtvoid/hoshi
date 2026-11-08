import { expect, test } from 'bun:test'
import { remember, stepHistory } from '../../src/lib/prompt-history'

const entries = ['first', 'second', 'third']

test('going back starts at the newest prompt and stops at the oldest', () => {
  expect(stepHistory(entries, null, 'back')).toEqual({
    position: 2,
    text: 'third'
  })
  expect(stepHistory(entries, 1, 'back')).toEqual({
    position: 0,
    text: 'first'
  })
  expect(stepHistory(entries, 0, 'back')).toBeNull()
  expect(stepHistory([], null, 'back')).toBeNull()
})

test('going forward past the newest prompt empties the composer', () => {
  expect(stepHistory(entries, 0, 'forward')).toEqual({
    position: 1,
    text: 'second'
  })
  expect(stepHistory(entries, 2, 'forward')).toEqual({
    position: null,
    text: ''
  })
  expect(stepHistory(entries, null, 'forward')).toBeNull()
})

test('remember skips repeats and blanks and keeps the newest', () => {
  expect(remember(['a', 'b'], ' b ', 3)).toEqual(['a', 'b'])
  expect(remember(['a', 'b'], '   ', 3)).toEqual(['a', 'b'])
  expect(remember(['a', 'b', 'c'], 'd', 3)).toEqual(['b', 'c', 'd'])
})
