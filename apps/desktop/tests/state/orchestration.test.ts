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
  activeTurnId: null,
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
      isError: false
    }
  }

  test('fills in a tool call when it completes', () => {
    const items = applyItemEvent('t', [tool], {
      type: 'toolCallCompleted',
      threadId: 't',
      itemId: 'tool',
      output: 'hello',
      isError: false
    })

    expect(items[0]?.content).toMatchObject({ output: 'hello' })
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
