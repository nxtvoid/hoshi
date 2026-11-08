import { describe, expect, test } from 'bun:test'
import {
  isPullRequestUrl,
  leadingLength,
  webLinkHost
} from '../../src/lib/links'

describe('webLinkHost', () => {
  test('reads the host of web links', () => {
    expect(webLinkHost('https://GitHub.com/nxtvoid/avzx/pull/3')).toBe(
      'github.com'
    )
    expect(webLinkHost('http://localhost:5173/x')).toBe('localhost')
  })

  test('ignores everything that is not a web link', () => {
    expect(webLinkHost(undefined)).toBeNull()
    expect(webLinkHost('#section')).toBeNull()
    expect(webLinkHost('mailto:a@b.c')).toBeNull()
    expect(webLinkHost('src/main.ts')).toBeNull()
  })
})

describe('leadingLength', () => {
  test('keeps the protocol of a URL next to its icon', () => {
    expect(leadingLength('https://github.com')).toBe('https://'.length)
  })

  test('keeps one character of a label', () => {
    expect(leadingLength('the docs')).toBe(1)
    expect(leadingLength('')).toBe(0)
  })
})

describe('isPullRequestUrl', () => {
  test('matches pull request pages and their tabs', () => {
    expect(isPullRequestUrl('https://github.com/nxtvoid/avzx/pull/3')).toBe(
      true
    )
    expect(isPullRequestUrl('https://github.com/o/r/pull/3/files')).toBe(true)
  })

  test('leaves other links alone', () => {
    expect(isPullRequestUrl('https://github.com/o/r/issues/3')).toBe(false)
    expect(isPullRequestUrl('https://github.com/o/r/pull/34x')).toBe(false)
    expect(isPullRequestUrl('https://gitlab.com/o/r/pull/3')).toBe(false)
  })
})
