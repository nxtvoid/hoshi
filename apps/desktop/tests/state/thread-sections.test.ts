import { describe, expect, test } from 'bun:test'
import type { Thread } from '../../src/bindings/Thread'
import type { PullRequest } from '../../src/bindings/PullRequest'
import {
  groupThreads,
  INACTIVE_AFTER_MS,
  inactiveAfterMs,
  inboxCount,
  inboxSections,
  matchesSearch,
  type SidebarQuery,
  sectionOf,
  sidebarSections,
  stepThread,
  visibleOrder,
  withDelegatedWork
} from '../../src/state/thread-sections'

const NOW = Date.parse('2026-11-07T12:00:00.000Z')

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
  parentThreadId: null,
  createdAt: new Date(NOW).toISOString(),
  updatedAt: new Date(NOW).toISOString(),
  ...changes
})

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
  additions: 1,
  deletions: 1,
  updatedAt: new Date(NOW).toISOString(),
  ...changes
})

const idleSince = (ms: number) => new Date(NOW - ms).toISOString()

describe('sectionOf', () => {
  test('archived wins over everything else', () => {
    expect(
      sectionOf(
        thread('t', { archived: true, status: 'error' }),
        undefined,
        NOW
      )
    ).toBe('done')
  })

  test('a pinned thread stays pinned while it needs attention', () => {
    const pinned = thread('t', { pinned: true, status: 'waitingForInput' })

    expect(sectionOf(pinned, undefined, NOW)).toBe('pinned')
  })

  test('waiting for input or failed needs attention', () => {
    const waiting = thread('t', { status: 'waitingForInput' })
    const failed = thread('t', { status: 'error' })

    expect(sectionOf(waiting, undefined, NOW)).toBe('attention')
    expect(sectionOf(failed, undefined, NOW)).toBe('attention')
  })

  test('an idle thread turns inactive after the threshold', () => {
    const recent = thread('t', { updatedAt: idleSince(INACTIVE_AFTER_MS) })
    const stale = thread('t', { updatedAt: idleSince(INACTIVE_AFTER_MS + 1) })

    expect(sectionOf(recent, undefined, NOW)).toBe('active')
    expect(sectionOf(stale, undefined, NOW)).toBe('inactive')
  })

  test('the threshold comes from the settings, in days', () => {
    const week = inactiveAfterMs(7)
    const fiveDays = thread('t', { updatedAt: idleSince(5 * 24 * 3600 * 1000) })

    expect(inactiveAfterMs(undefined)).toBe(INACTIVE_AFTER_MS)
    expect(inactiveAfterMs(0)).toBe(INACTIVE_AFTER_MS)
    expect(sectionOf(fiveDays, undefined, NOW)).toBe('inactive')
    expect(sectionOf(fiveDays, undefined, NOW, week)).toBe('active')
  })

  test('a working thread is active even with a pull request', () => {
    const working = thread('t', { status: 'working' })

    expect(sectionOf(working, pullRequest(), NOW)).toBe('active')
  })

  test('an open pull request is in review, however old the thread', () => {
    const old = thread('t', { updatedAt: idleSince(INACTIVE_AFTER_MS * 5) })

    expect(sectionOf(old, pullRequest(), NOW)).toBe('review')
    expect(sectionOf(old, pullRequest({ state: 'draft' }), NOW)).toBe('review')
  })

  test('failing checks or unresolved comments need attention', () => {
    const failing = pullRequest({ checks: 'failing' })
    const commented = pullRequest({ unresolvedComments: 2 })

    expect(sectionOf(thread('t'), failing, NOW)).toBe('attention')
    expect(sectionOf(thread('t'), commented, NOW)).toBe('attention')
  })

  test('a merged or closed pull request is done', () => {
    const merged = pullRequest({ state: 'merged', unresolvedComments: 3 })

    expect(sectionOf(thread('t'), merged, NOW)).toBe('done')
    expect(sectionOf(thread('t'), pullRequest({ state: 'closed' }), NOW)).toBe(
      'done'
    )
  })
})

describe('groupThreads', () => {
  test('orders sections and keeps the order of threads inside each', () => {
    const threads = [
      thread('a'),
      thread('old', { updatedAt: idleSince(INACTIVE_AFTER_MS * 2) }),
      thread('r'),
      thread('b'),
      thread('p', { pinned: true }),
      thread('x', { archived: true })
    ]

    const sections = groupThreads(threads, { r: pullRequest() }, NOW).map(
      ({ id, threads }) => [id, threads.map((candidate) => candidate.id)]
    )

    expect(sections).toEqual([
      ['pinned', ['p']],
      ['active', ['a', 'b']],
      ['review', ['r']],
      ['inactive', ['old']],
      ['done', ['x']]
    ])
  })
})

const ids = (sections: { id: string; threads: Thread[] }[]) =>
  sections.map((section) => [
    section.id,
    section.threads.map((each) => each.id)
  ])

describe('matchesSearch', () => {
  const fix = thread('t', { title: 'Fix login', branch: 'feat/auth' })

  test('matches the title, the branch and the pull request number', () => {
    expect(matchesSearch(fix, undefined, 'login')).toBe(true)
    expect(matchesSearch(fix, undefined, 'auth')).toBe(true)
    expect(matchesSearch(fix, pullRequest({ number: 128 }), '#128')).toBe(true)
    expect(matchesSearch(fix, undefined, '#128')).toBe(false)
  })

  test('needs every word', () => {
    expect(matchesSearch(fix, undefined, 'fix auth')).toBe(true)
    expect(matchesSearch(fix, undefined, 'fix billing')).toBe(false)
  })
})

describe('sidebarSections', () => {
  const query: SidebarQuery = {
    view: 'project',
    projectId: 'p',
    search: '',
    filter: 'all',
    sort: 'activity'
  }
  const threads = [
    thread('root', { updatedAt: '2026-11-07T10:00:00.000Z' }),
    thread('tree', {
      worktreePath: '/w',
      updatedAt: '2026-11-07T11:00:00.000Z'
    }),
    thread('other', { projectId: 'q', status: 'working' })
  ]

  test('lists the selected project, newest first', () => {
    expect(ids(sidebarSections(threads, {}, query, NOW))).toEqual([
      ['active', ['tree', 'root']]
    ])
  })

  test('filters worktrees and pull requests', () => {
    expect(
      ids(sidebarSections(threads, {}, { ...query, filter: 'worktrees' }, NOW))
    ).toEqual([['active', ['tree']]])
    expect(
      ids(
        sidebarSections(
          threads,
          { root: pullRequest() },
          { ...query, filter: 'pullRequests' },
          NOW
        )
      )
    ).toEqual([['review', ['root']]])
  })

  test('the inbox crosses projects', () => {
    expect(
      ids(sidebarSections(threads, {}, { ...query, view: 'inbox' }, NOW))
    ).toEqual([['active', ['other']]])
  })

  test('chats list apart from every project, and only there', () => {
    const withChat = [
      ...threads,
      thread('chat', { projectId: null, updatedAt: '2026-11-07T09:00:00.000Z' })
    ]
    expect(
      ids(sidebarSections(withChat, {}, { ...query, view: 'chats' }, NOW))
    ).toEqual([['active', ['chat']]])
    expect(ids(sidebarSections(withChat, {}, query, NOW))).toEqual([
      ['active', ['tree', 'root']]
    ])
    expect(
      ids(
        sidebarSections(
          withChat,
          {},
          { ...query, view: 'project', projectId: null },
          NOW
        )
      )
    ).toEqual([])
  })
})

describe('inboxSections', () => {
  test('keeps what needs the user, working threads and reviews', () => {
    const sections = inboxSections(
      [
        thread('waiting', { status: 'waitingForInput', pinned: true }),
        thread('working', { status: 'working' }),
        thread('idle'),
        thread('review'),
        thread('archived', { archived: true, status: 'error' })
      ],
      { review: pullRequest() },
      NOW
    )
    expect(ids(sections)).toEqual([
      ['attention', ['waiting']],
      ['active', ['working']],
      ['review', ['review']]
    ])
    expect(sections[0]?.threads[0]?.pinned).toBe(true)
  })
})

describe('visibleOrder', () => {
  const sections = [
    { id: 'active' as const, threads: [thread('a'), thread('b')] },
    { id: 'done' as const, threads: [thread('c'), thread('d')] }
  ]

  test('skips folded sections except the selected thread', () => {
    expect(visibleOrder(sections, (id) => id === 'active', 'd')).toEqual([
      'a',
      'b',
      'd'
    ])
  })
})

describe('stepThread', () => {
  test('moves and wraps around', () => {
    expect(stepThread(['a', 'b', 'c'], 'b', 1)).toBe('c')
    expect(stepThread(['a', 'b', 'c'], 'c', 1)).toBe('a')
    expect(stepThread(['a', 'b', 'c'], 'a', -1)).toBe('c')
  })

  test('starts at an end when nothing is selected', () => {
    expect(stepThread(['a', 'b'], null, 1)).toBe('a')
    expect(stepThread(['a', 'b'], 'gone', -1)).toBe('b')
    expect(stepThread([], null, 1)).toBeNull()
  })
})

describe('inbox completed turns', () => {
  test('a finished turn not seen yet gets its own section before reviews', () => {
    const sections = inboxSections(
      [
        thread('seen'),
        thread('finished'),
        thread('reviewed'),
        thread('waiting', { status: 'waitingForInput' })
      ],
      { finished: pullRequest(), reviewed: pullRequest() },
      NOW,
      { finished: true }
    )
    expect(ids(sections)).toEqual([
      ['attention', ['waiting']],
      ['completed', ['finished']],
      ['review', ['reviewed']]
    ])
    expect(inboxCount(sections)).toBe(2)
  })
})

describe('withDelegatedWork', () => {
  test('lists only the threads the user started, with their tasks’ work on them', () => {
    const threads = [
      thread('parent'),
      thread('task', { parentThreadId: 'parent', status: 'waitingForInput' }),
      thread('other', { status: 'working' }),
      thread('done', { parentThreadId: 'other', status: 'idle' })
    ]

    const listed = withDelegatedWork(threads)

    expect(listed.map((each) => [each.id, each.status])).toEqual([
      ['parent', 'waitingForInput'],
      ['other', 'working']
    ])
  })

  test('a task of a task works for the thread the user started', () => {
    const listed = withDelegatedWork([
      thread('parent'),
      thread('task', { parentThreadId: 'parent', status: 'working' }),
      thread('deeper', { parentThreadId: 'task', status: 'waitingForInput' })
    ])

    expect(listed.map((each) => [each.id, each.status])).toEqual([
      ['parent', 'waitingForInput']
    ])
  })

  test('a task whose thread is gone is listed on its own', () => {
    const listed = withDelegatedWork([
      thread('task', { parentThreadId: 'deleted', status: 'working' })
    ])

    expect(listed.map((each) => each.id)).toEqual(['task'])
  })

  test('a failed thread keeps showing its own failure', () => {
    const listed = withDelegatedWork([
      thread('parent', { status: 'error' }),
      thread('task', { parentThreadId: 'parent', status: 'working' })
    ])

    expect(listed.map((each) => each.status)).toEqual(['error'])
  })
})
