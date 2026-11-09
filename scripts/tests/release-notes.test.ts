import { describe, expect, test } from 'bun:test'
import { changeBullet, releaseNotes } from '../release-notes'

describe('changeBullet', () => {
  test('drops the Conventional Commit prefix and capitalizes', () => {
    expect(changeBullet('feat: appearance settings with themes (#9)')).toBe(
      '- Appearance settings with themes (#9)'
    )
    expect(changeBullet('fix(linux): paste images (#3)')).toBe(
      '- Paste images (#3)'
    )
    expect(changeBullet('perf!: faster startup')).toBe('- Faster startup')
  })

  test('keeps subjects without a prefix', () => {
    expect(changeBullet('Sidebar layout and themes')).toBe(
      '- Sidebar layout and themes'
    )
  })

  test('skips release and merge commits', () => {
    expect(changeBullet('chore: release v0.0.4')).toBeNull()
    expect(changeBullet('Merge pull request #1 from nxtvoid/next')).toBeNull()
    expect(changeBullet('chore: check pull requests in CI (#2)')).toBe(
      '- Check pull requests in CI (#2)'
    )
  })
})

describe('releaseNotes', () => {
  test('lists the changes above the download line', () => {
    expect(
      releaseNotes(['chore: release v0.0.5', 'feat: one (#2)', 'fix: two'])
    ).toBe(
      '- One (#2)\n- Two\n\nDownload the installer for your system from the assets below.'
    )
  })

  test('is only the download line without changes', () => {
    expect(releaseNotes([])).toBe(
      'Download the installer for your system from the assets below.'
    )
  })
})
