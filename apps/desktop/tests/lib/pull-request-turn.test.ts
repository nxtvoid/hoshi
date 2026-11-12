import { describe, expect, test } from 'bun:test'
import type { PullRequest } from '../../src/bindings/PullRequest'
import { pullRequestTurn } from '../../src/lib/pull-request-turn'

const pullRequest = (changes: Partial<PullRequest> = {}): PullRequest => ({
  forge: 'bitbucket',
  number: 7,
  title: 'Fix',
  url: 'https://bitbucket.org/o/r/pull-requests/7',
  author: 'me',
  headBranch: 'fix',
  headSha: null,
  baseBranch: 'main',
  state: 'open',
  checks: null,
  unresolvedComments: 0,
  openTasks: 0,
  changesRequested: false,
  turn: null,
  conflicts: false,
  additions: 1,
  deletions: 1,
  updatedAt: '2026-11-07T12:00:00Z',
  ...changes
})

describe('pullRequestTurn', () => {
  test('a request for changes nobody dated still waits on the author', () => {
    expect(pullRequestTurn(pullRequest({ changesRequested: true }))).toEqual({
      kind: 'author',
      ask: null,
      threads: 0,
      tasks: 0
    })
  })

  test('an answered request waits on the reviewer until a thread reopens it', () => {
    const answered = pullRequest({
      changesRequested: true,
      turn: {
        kind: 'reviewers',
        waitingOn: ['luis'],
        repliedAt: '2026-11-07T11:00:00Z'
      }
    })

    expect(pullRequestTurn({ ...answered, openTasks: 2 })).toEqual({
      kind: 'reviewers',
      waitingOn: ['luis'],
      repliedAt: '2026-11-07T11:00:00Z',
      tasks: 2
    })
    expect(pullRequestTurn({ ...answered, unresolvedComments: 1 })?.kind).toBe(
      'author'
    )
  })

  test('nothing waits on anyone once it is merged or nobody spoke', () => {
    expect(pullRequestTurn(pullRequest())).toBeNull()
    expect(
      pullRequestTurn(pullRequest({ state: 'merged', unresolvedComments: 3 }))
    ).toBeNull()
  })
})
