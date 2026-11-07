import { describe, expect, test } from 'bun:test'
import type { Thread } from '../../src/bindings/Thread'
import type { Project } from '../../src/bindings/Project'
import { applyEvent, folderName, samePath } from '../../src/state/orchestration'

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
