import { describe, expect, test } from 'bun:test'
import { detectTrigger } from '../../src/lib/composer-trigger'

describe('detectTrigger', () => {
  test('finds an @ mention anywhere in the prompt', () => {
    const text = 'look at @src/ma'
    expect(detectTrigger(text, text.length)).toEqual({
      kind: 'path',
      query: 'src/ma',
      start: 8,
      end: 15
    })
  })

  test('reads the query up to the cursor but replaces the whole token', () => {
    expect(detectTrigger('@src/main.rs please', 4)).toEqual({
      kind: 'path',
      query: 'src',
      start: 0,
      end: 12
    })
  })

  test('ignores an @ inside a word, like an email', () => {
    const text = 'mail me@example.com'
    expect(detectTrigger(text, text.length)).toBeNull()
  })

  test('offers commands only at the start of the prompt', () => {
    expect(detectTrigger('  /comp', 7)).toEqual({
      kind: 'command',
      query: 'comp',
      start: 2,
      end: 7
    })
    expect(detectTrigger('run /comp', 9)).toBeNull()
  })

  test('a path is not a command', () => {
    expect(detectTrigger('/usr/bin', 8)).toBeNull()
  })

  test('nothing is offered once the token ends', () => {
    expect(detectTrigger('@src/main.rs ', 13)).toBeNull()
  })
})

describe('pull request and skill triggers', () => {
  test('# offers pull requests by number or words', () => {
    expect(detectTrigger('fix #12', 7)).toEqual({
      kind: 'pullRequest',
      query: '12',
      start: 4,
      end: 7
    })
    expect(detectTrigger('#', 1)?.kind).toBe('pullRequest')
    expect(detectTrigger('## Title', 2)).toBeNull()
  })

  test('$ offers skills but not prices', () => {
    expect(detectTrigger('use $pd', 7)).toEqual({
      kind: 'skill',
      query: 'pd',
      start: 4,
      end: 7
    })
    expect(detectTrigger('costs $5', 8)).toBeNull()
  })
})
