import { describe, expect, test } from 'bun:test'
import type { Thread } from '../../src/bindings/Thread'
import type { Project } from '../../src/bindings/Project'
import { startupSelection } from '../../src/state/startup'

const project = (id: string): Project => ({
  id,
  title: id,
  workspaceRoot: `/code/${id}`,
  createdAt: '2026-11-01T00:00:00.000Z',
  updatedAt: '2026-11-01T00:00:00.000Z'
})

const thread = (id: string, projectId: string | null): Thread => ({
  id,
  projectId,
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
  updatedAt: '2026-11-01T00:00:00.000Z'
})

const projects = [project('a'), project('b')]
const threads = [thread('t1', 'a'), thread('t2', 'b'), thread('c1', null)]

describe('startupSelection', () => {
  test('the inbox opens with no thread, keeping the last project on the rail', () => {
    expect(
      startupSelection(
        'inbox',
        { projectId: 'b', threadId: 't2' },
        projects,
        threads
      )
    ).toEqual({ sidebarView: 'inbox', projectId: 'b', threadId: null })
  })

  test('a lone project without chats opens in place of the inbox', () => {
    expect(
      startupSelection(
        'inbox',
        { projectId: 'a', threadId: 't1' },
        [project('a')],
        [thread('t1', 'a')]
      )
    ).toEqual({ sidebarView: 'project', projectId: 'a', threadId: null })
    expect(
      startupSelection(
        'inbox',
        { projectId: 'a', threadId: null },
        [project('a')],
        [thread('t1', 'a'), thread('c1', null)]
      )
    ).toEqual({ sidebarView: 'inbox', projectId: 'a', threadId: null })
    expect(
      startupSelection('inbox', { projectId: null, threadId: null }, [], [])
    ).toEqual({ sidebarView: 'inbox', projectId: null, threadId: null })
  })

  test('the last project reopens with its thread', () => {
    expect(
      startupSelection(
        'lastProject',
        { projectId: 'b', threadId: 't2' },
        projects,
        threads
      )
    ).toEqual({ sidebarView: 'project', projectId: 'b', threadId: 't2' })
  })

  test('a chat left open reopens among the chats', () => {
    expect(
      startupSelection(
        'lastProject',
        { projectId: 'b', threadId: 'c1' },
        projects,
        threads
      )
    ).toEqual({ sidebarView: 'chats', projectId: 'b', threadId: 'c1' })
  })

  test('what is gone falls back to the first project', () => {
    expect(
      startupSelection(
        'lastProject',
        { projectId: 'gone', threadId: 't2' },
        projects,
        threads
      )
    ).toEqual({ sidebarView: 'project', projectId: 'a', threadId: null })
    expect(
      startupSelection(
        'lastProject',
        { projectId: null, threadId: null },
        [],
        []
      )
    ).toEqual({ sidebarView: 'project', projectId: null, threadId: null })
  })
})
