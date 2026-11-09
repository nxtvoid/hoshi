import { describe, expect, test } from 'bun:test'
import type { GitStatus } from '../../src/bindings/GitStatus'
import {
  menuBlockers,
  pushesToDefaultBranch,
  quickAction
} from '../../src/lib/git-actions'

const status = (change: Partial<GitStatus> = {}): GitStatus => ({
  branch: 'hoshi/login',
  defaultBranch: 'main',
  hasChanges: false,
  hasUpstream: false,
  hasRemote: true,
  ahead: 0,
  behind: 0,
  aheadOfDefault: 0,
  ...change
})

const pick = (value: GitStatus, pullRequestOpen = false) => {
  const { label, blocker } = quickAction(value, pullRequestOpen)
  return blocker ? `${label}!${blocker}` : label
}

describe('quickAction', () => {
  test('changes go as far as they can: a pull request, or a push once one exists', () => {
    expect(pick(status({ hasChanges: true }))).toBe('commitPushPullRequest')
    expect(pick(status({ hasChanges: true }), true)).toBe('commitPush')
    expect(pick(status({ hasChanges: true, branch: 'main' }))).toBe(
      'commitPush'
    )
    expect(pick(status({ hasChanges: true, hasRemote: false }))).toBe('commit')
    expect(
      pick(status({ hasChanges: true, hasUpstream: true, behind: 2 }))
    ).toBe('commit')
  })

  test('commits not pushed yet are pushed, with a pull request when there is none', () => {
    expect(pick(status({ aheadOfDefault: 1 }))).toBe('pushCreatePullRequest')
    expect(pick(status({ aheadOfDefault: 1 }), true)).toBe('push')
    expect(
      pick(status({ hasUpstream: true, ahead: 1, aheadOfDefault: 1 }))
    ).toBe('pushCreatePullRequest')
    expect(
      pick(status({ hasUpstream: true, ahead: 1, aheadOfDefault: 1 }), true)
    ).toBe('push')
  })

  test('behind pulls, diverged waits, and pushed work only opens a pull request', () => {
    expect(pick(status({ hasUpstream: true, behind: 1 }))).toBe('pull')
    expect(pick(status({ hasUpstream: true, behind: 1, ahead: 1 }))).toBe(
      'pull!diverged'
    )
    expect(pick(status({ hasUpstream: true, aheadOfDefault: 2 }))).toBe(
      'createPullRequest'
    )
    expect(pick(status({ hasUpstream: true, aheadOfDefault: 2 }), true)).toBe(
      'commit!upToDate'
    )
  })

  test('nothing to do, no remote or no branch block the button with a reason', () => {
    expect(pick(status())).toBe('push!noCommits')
    expect(pick(status({ hasRemote: false }))).toBe('commit!noRemote')
    expect(pick(status({ branch: null, hasChanges: true }))).toBe(
      'commit!detached'
    )
  })
})

describe('menuBlockers', () => {
  test('says why each entry waits', () => {
    expect(menuBlockers(status())).toEqual({
      commit: 'clean',
      push: 'noCommits',
      createPullRequest: 'noCommits'
    })
    expect(
      menuBlockers(status({ hasChanges: true, aheadOfDefault: 1 }))
    ).toEqual({ commit: null, push: 'uncommitted', createPullRequest: null })
  })
})

test('pushing to the default branch is the one that asks first', () => {
  const main = status({ branch: 'main', hasChanges: true })
  expect(pushesToDefaultBranch(main, 'commitPush')).toBe(true)
  expect(pushesToDefaultBranch(main, 'commit')).toBe(false)
  expect(pushesToDefaultBranch(status(), 'push')).toBe(false)
})
