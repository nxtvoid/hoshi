import type { Thread } from '@/bindings/Thread'
import type { ThreadItem } from '@/bindings/ThreadItem'
import { isSubagentCall, subagentInput } from './tool-calls'

export type AgentState = 'running' | 'waiting' | 'done' | 'stopped' | 'failed'

/** An agent that worked for a thread: a task it delegated, or a subagent of its turns. */
export type ThreadAgent = {
  kind: 'task' | 'subagent'
  /** The child thread of a task; the call that started a subagent. */
  id: string
  title: string
  state: AgentState
  /** When it started, as RFC 3339. */
  at: string
}

function taskState(thread: Thread): AgentState {
  switch (thread.status) {
    case 'working':
      return 'running'
    case 'waitingForInput':
      return 'waiting'
    case 'error':
      return 'failed'
    case 'idle':
      return 'done'
  }
}

/**
 * The thread's agents, newest first: the tasks it delegated, and the subagents its loaded
 * transcript shows.
 */
export function threadAgents(
  threadId: string,
  threads: Thread[],
  items: ThreadItem[]
): ThreadAgent[] {
  const tasks = threads
    .filter((thread) => thread.parentThreadId === threadId)
    .map(
      (thread): ThreadAgent => ({
        kind: 'task',
        id: thread.id,
        title: thread.title,
        state: taskState(thread),
        at: thread.createdAt
      })
    )
  const subagents = items
    .filter((item) => item.threadId === threadId && isSubagentCall(item))
    .map((item): ThreadAgent => {
      const content = item.content.kind === 'toolCall' ? item.content : null
      const { description, kind } = subagentInput(content?.input ?? '')
      return {
        kind: 'subagent',
        id: item.id,
        title: description || kind,
        state: content?.interrupted
          ? 'stopped'
          : content?.output === null
            ? 'running'
            : content?.isError
              ? 'failed'
              : 'done',
        at: item.createdAt
      }
    })
  return [...tasks, ...subagents].sort((a, b) => b.at.localeCompare(a.at))
}
