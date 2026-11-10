import { describe, expect, test } from 'bun:test'
import type { Thread } from '../../src/bindings/Thread'
import type { Project } from '../../src/bindings/Project'
import type { PullRequest } from '../../src/bindings/PullRequest'
import {
  PALETTE_ACTIONS,
  type PaletteSource,
  scoreField,
  searchPalette
} from '../../src/state/palette'

const thread = (id: string, changes: Partial<Thread> = {}): Thread => ({
  id,
  projectId: 'p',
  title: id,
  status: 'idle',
  branch: null,
  worktreePath: null,
  sessionId: null,
  model: null,
  effort: null,
  fastMode: false,
  ultracode: false,
  accessMode: 'acceptEdits',
  planMode: false,
  activeTurnId: null,
  archived: false,
  pinned: false,
  worktreeReleased: false,
  worktreeAdopted: false,
  createdAt: '2026-11-01T00:00:00.000Z',
  updatedAt: '2026-11-01T00:00:00.000Z',
  ...changes
})

const project = (id: string, title: string): Project => ({
  id,
  title,
  workspaceRoot: `/code/${title}`,
  createdAt: '2026-11-01T00:00:00.000Z',
  updatedAt: '2026-11-01T00:00:00.000Z'
})

const pullRequest = (number: number, title: string): PullRequest => ({
  forge: 'github',
  number,
  title,
  url: `https://github.com/o/r/pull/${number}`,
  author: 'me',
  headBranch: 'fix',
  headSha: null,
  baseBranch: 'main',
  state: 'open',
  checks: null,
  unresolvedComments: 0,
  additions: 0,
  deletions: 0,
  updatedAt: '2026-11-01T00:00:00.000Z'
})

const source = (changes: Partial<PaletteSource> = {}): PaletteSource => ({
  threads: [],
  projects: [project('p', 'hoshi'), project('q', 'website')],
  pullRequests: {},
  actionLabels: {
    newThread: 'New thread',
    newThreadFromPullRequest: 'New thread from pull request',
    addProject: 'Add project',
    newNote: 'New note',
    notes: 'Notes',
    inbox: 'Inbox',
    settings: 'Settings',
    usage: 'Usage'
  },
  ...changes
})

const threadIds = (query: string, changes: Partial<PaletteSource>) =>
  searchPalette(query, source(changes)).threads.map((each) => each.id)

describe('scoreField', () => {
  test('ranks a prefix over a word start over a substring', () => {
    expect(scoreField('Login form', 'log')).toBe(3)
    expect(scoreField('feat/login', 'log')).toBe(2)
    expect(scoreField('catalog', 'log')).toBe(1)
    expect(scoreField('catalog', 'xyz')).toBe(0)
  })
})

describe('searchPalette', () => {
  test('with no query shows the latest open threads and every action', () => {
    const results = searchPalette(
      '  ',
      source({
        threads: [
          thread('old'),
          thread('new', { updatedAt: '2026-11-05T00:00:00.000Z' }),
          thread('gone', { archived: true })
        ]
      })
    )
    expect(results.threads.map((each) => each.id)).toEqual(['new', 'old'])
    expect(results.actions).toEqual(PALETTE_ACTIONS)
    expect(results.projects).toEqual([])
  })

  test('finds threads by pull request, branch and project', () => {
    expect(
      threadIds('#42', {
        threads: [thread('a'), thread('b')],
        pullRequests: { a: pullRequest(42, 'Fix crash') }
      })
    ).toEqual(['a'])
    expect(
      threadIds('auth', { threads: [thread('b', { branch: 'fix/auth' })] })
    ).toEqual(['b'])
    expect(
      threadIds('website', { threads: [thread('c', { projectId: 'q' })] })
    ).toEqual(['c'])
  })

  test('ranks better matches first and needs every word', () => {
    expect(
      threadIds('log', {
        threads: [thread('catalog'), thread('login', { title: 'Login page' })]
      })
    ).toEqual(['login', 'catalog'])
    expect(threadIds('login billing', { threads: [thread('Login')] })).toEqual(
      []
    )
  })

  test('matches projects and actions by name', () => {
    expect(searchPalette('set', source()).actions).toEqual(['settings'])
    expect(
      searchPalette('hos', source()).projects.map((each) => each.id)
    ).toEqual(['p'])
  })
})
