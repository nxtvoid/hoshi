import { describe, expect, test } from 'bun:test'
import { mergedCleanup } from '@/state/merged-cleanup'
import type { PullRequest } from '@/bindings/PullRequest'

const pullRequest = (state: PullRequest['state']): PullRequest => ({
  forge: 'github',
  number: 7,
  title: 'Work',
  url: 'https://github.com/o/r/pull/7',
  author: null,
  headBranch: 'feature',
  headSha: null,
  baseBranch: 'main',
  state,
  checks: null,
  unresolvedComments: 0,
  additions: 0,
  deletions: 0,
  updatedAt: '2026-11-07T12:00:00Z'
})

describe('mergedCleanup', () => {
  test('waits for the merge', () => {
    const thread = {
      worktreePath: '/wt',
      branch: 'feature',
      worktreeAdopted: false
    }
    expect(mergedCleanup(thread, pullRequest('open'))).toBeNull()
    expect(mergedCleanup(thread, undefined)).toBeNull()
  })

  test('removes a worktree', () => {
    const thread = {
      worktreePath: '/wt',
      branch: 'feature',
      worktreeAdopted: false
    }
    expect(mergedCleanup(thread, pullRequest('merged'))).toBe('worktree')
  })

  test('switches a root thread back only while it is on the merged branch', () => {
    const merged = pullRequest('merged')
    expect(
      mergedCleanup(
        { worktreePath: null, branch: 'feature', worktreeAdopted: false },
        merged
      )
    ).toBe('root')
    expect(
      mergedCleanup(
        { worktreePath: null, branch: 'main', worktreeAdopted: false },
        merged
      )
    ).toBeNull()
  })

  test('leaves an adopted worktree alone', () => {
    const thread = {
      worktreePath: '/wt',
      branch: 'feature',
      worktreeAdopted: true
    }
    expect(mergedCleanup(thread, pullRequest('merged'))).toBeNull()
  })
})
