import { describe, expect, test } from 'bun:test'
import type { PullRequest } from '../../src/bindings/PullRequest'
import { pullRequestProblem } from '../../src/lib/pull-request-problem'

const pullRequest = (changes: Partial<PullRequest> = {}): PullRequest => ({
  forge: 'github',
  number: 42,
  title: 'Fix',
  url: 'https://github.com/o/r/pull/42',
  author: 'me',
  headBranch: 'fix',
  headSha: null,
  baseBranch: 'main',
  state: 'open',
  checks: 'passing',
  unresolvedComments: 0,
  changesRequested: false,
  conflicts: false,
  additions: 1,
  deletions: 1,
  updatedAt: '2026-11-07T12:00:00.000Z',
  ...changes
})

describe('pullRequestProblem', () => {
  test('names the worst problem first', () => {
    expect(
      pullRequestProblem(
        pullRequest({
          conflicts: true,
          checks: 'failing',
          changesRequested: true
        })
      )
    ).toBe('conflicts')
    expect(
      pullRequestProblem(
        pullRequest({ checks: 'failing', changesRequested: true })
      )
    ).toBe('checksFailing')
    expect(pullRequestProblem(pullRequest({ changesRequested: true }))).toBe(
      'changesRequested'
    )
    expect(pullRequestProblem(pullRequest())).toBeNull()
  })

  test('only open pull requests have one', () => {
    for (const state of ['draft', 'merged', 'closed'] as const) {
      expect(pullRequestProblem(pullRequest({ state, conflicts: true }))).toBe(
        null
      )
    }
  })
})
