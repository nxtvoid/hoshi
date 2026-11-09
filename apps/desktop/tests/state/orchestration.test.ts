import { describe, expect, test } from 'bun:test'
import type { Thread } from '../../src/bindings/Thread'
import type { Project } from '../../src/bindings/Project'
import type { ThreadItem } from '../../src/bindings/ThreadItem'
import {
  applyEvent,
  applyItemEvent,
  folderName,
  samePath
} from '../../src/state/orchestration'

const NOW = '2026-11-07T12:00:00.000Z'

const project = (id: string): Project => ({
  id,
  title: id,
  workspaceRoot: `/repo/${id}`,
  createdAt: NOW,
  updatedAt: NOW
})

const thread = (id: string, projectId: string): Thread => ({
  id,
  projectId,
  title: '',
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
  createdAt: NOW,
  updatedAt: NOW
})

describe('applyEvent', () => {
  test('removing a project drops its threads and keeps the rest', () => {
    const state = {
      projects: [project('p'), project('q')],
      threads: [thread('t1', 'p'), thread('t2', 'q')]
    }

    const next = applyEvent(state, { type: 'projectRemoved', projectId: 'p' })

    expect(next.projects.map((item) => item.id)).toEqual(['q'])
    expect(next.threads.map((item) => item.id)).toEqual(['t2'])
  })

  test('deleting a thread leaves projects untouched', () => {
    const state = { projects: [project('p')], threads: [thread('t', 'p')] }

    const next = applyEvent(state, { type: 'threadDeleted', threadId: 't' })

    expect(next.threads).toEqual([])
    expect(next.projects).toBe(state.projects)
  })

  test('a turn marks the thread working, then failed turns mark it as an error', () => {
    const state = { projects: [project('p')], threads: [thread('t', 'p')] }

    const started = applyEvent(state, {
      type: 'turnStarted',
      threadId: 't',
      turnId: 'm1',
      at: NOW
    })
    const failed = applyEvent(started, {
      type: 'turnCompleted',
      threadId: 't',
      turnId: 'm1',
      outcome: { type: 'failed', reason: { code: 'providerDisabled' } },
      at: NOW
    })

    expect(started.threads[0]).toMatchObject({
      status: 'working',
      activeTurnId: 'm1'
    })
    expect(failed.threads[0]).toMatchObject({
      status: 'error',
      activeTurnId: null
    })
  })

  test('a thread adopts a worktree its agent entered, then leaves it', () => {
    const state = { projects: [project('p')], threads: [thread('t', 'p')] }

    const adopted = applyEvent(state, {
      type: 'threadWorktreeAttached',
      threadId: 't',
      worktreePath: '/repo-pr',
      branch: 'feat/pr',
      adopted: true
    })
    const left = applyEvent(adopted, {
      type: 'threadWorktreeDetached',
      threadId: 't',
      branch: 'main'
    })

    expect(adopted.threads[0]).toMatchObject({
      worktreePath: '/repo-pr',
      branch: 'feat/pr',
      worktreeAdopted: true
    })
    expect(left.threads[0]).toMatchObject({
      worktreePath: null,
      branch: 'main',
      worktreeAdopted: false
    })
  })

  test('a released worktree comes back, and a cleaned up thread loses it', () => {
    const state = {
      projects: [project('p')],
      threads: [
        {
          ...thread('t', 'p'),
          worktreePath: '/w/t',
          branch: 'feat/x',
          sessionId: 's1'
        }
      ]
    }

    const released = applyEvent(state, {
      type: 'threadWorktreeReleased',
      threadId: 't'
    })
    const restored = applyEvent(released, {
      type: 'threadWorktreeRestored',
      threadId: 't'
    })
    const cleaned = applyEvent(released, {
      type: 'threadCleanedUp',
      threadId: 't'
    })

    expect(released.threads[0]).toMatchObject({
      worktreeReleased: true,
      worktreePath: '/w/t',
      branch: 'feat/x'
    })
    expect(restored.threads[0]?.worktreeReleased).toBe(false)
    expect(cleaned.threads[0]).toMatchObject({
      worktreePath: null,
      branch: null,
      worktreeReleased: false,
      sessionId: null
    })
  })
})

describe('rewinds', () => {
  test('a rewind to the start drops the session; one to a message pins it', () => {
    const state = {
      projects: [project('p')],
      threads: [
        { ...thread('t', 'p'), sessionId: 's', conversationCursor: 'u2' }
      ]
    }

    const pinned = applyEvent(state, {
      type: 'threadRewound',
      threadId: 't',
      messageId: 'm2',
      resumeAt: 'u1'
    })
    const restarted = applyEvent(state, {
      type: 'threadRewound',
      threadId: 't',
      messageId: 'm1',
      resumeAt: null
    })

    expect(pinned.threads[0]).toMatchObject({
      sessionId: 's',
      conversationCursor: 'u1',
      resumeAt: 'u1'
    })
    expect(restarted.threads[0]).toMatchObject({
      sessionId: null,
      resumeAt: null
    })
  })

  test('the transcript loses the message and what followed', () => {
    const message = (id: string): ThreadItem => ({
      id,
      threadId: 't',
      createdAt: NOW,
      content: { kind: 'assistantMessage', text: id }
    })
    const items = [message('m1'), message('a1'), message('m2'), message('a2')]

    const kept = applyItemEvent('t', items, {
      type: 'threadRewound',
      threadId: 't',
      messageId: 'm2',
      resumeAt: 'u1'
    })

    expect(kept.map((item) => item.id)).toEqual(['m1', 'a1'])
  })
})

describe('applyItemEvent', () => {
  const tool: ThreadItem = {
    id: 'tool',
    threadId: 't',
    createdAt: NOW,
    content: {
      kind: 'toolCall',
      name: 'Read',
      input: '{}',
      output: null,
      isError: false,
      detail: null,
      outputOmitted: false,
      interrupted: false
    }
  }

  test('fills in a tool call when it completes', () => {
    const detail = {
      kind: 'fileChange' as const,
      path: '/repo/a.txt',
      additions: 1,
      deletions: 0,
      diff: '@@ -0,0 +1,1 @@\n+a'
    }
    const items = applyItemEvent('t', [tool], {
      type: 'toolCallCompleted',
      threadId: 't',
      itemId: 'tool',
      output: 'hello',
      isError: false,
      detail
    })

    expect(items[0]?.content).toMatchObject({ output: 'hello', detail })
  })

  test('stops a tool call whose turn ended first', () => {
    const items = applyItemEvent('t', [tool], {
      type: 'toolCallInterrupted',
      threadId: 't',
      itemId: 'tool'
    })

    expect(items[0]?.content).toMatchObject({ output: null, interrupted: true })
  })

  test('replaces an item that arrives twice and ignores other threads', () => {
    const again = { ...tool, content: { ...tool.content, input: '{"a":1}' } }

    const items = applyItemEvent('t', [tool], {
      type: 'itemAdded',
      item: again
    })
    const other = applyItemEvent('x', items, { type: 'itemAdded', item: again })

    expect(items).toEqual([again])
    expect(other).toBe(items)
  })
})

describe('paths', () => {
  test.each([
    ['C:\\Repo\\Hoshi', 'c:/repo/hoshi/', true],
    ['/repo/a', '/repo/b', false]
  ])('samePath(%s, %s) is %p', (a, b, expected) => {
    expect(samePath(a, b)).toBe(expected)
  })

  test.each([
    ['C:\\Users\\me\\projects\\hoshi', 'hoshi'],
    ['/home/me/notes/', 'notes']
  ])('folderName(%s) is %s', (path, expected) => {
    expect(folderName(path)).toBe(expected)
  })
})
