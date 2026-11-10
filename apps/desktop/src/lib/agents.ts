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

/**
 * A task that waits for the user's changes to its plan: still planning, at rest, its
 * answer not given back.
 */
export function isPlanningWithUser(
  thread: Thread,
  pendingTasks: Record<string, true>
) {
  return (
    thread.parentThreadId !== null &&
    thread.planMode &&
    thread.status === 'idle' &&
    pendingTasks[thread.id] === true
  )
}

/** How a delegated task stands, read from its thread. */
export function taskState(
  thread: Thread,
  pendingTasks: Record<string, true>
): AgentState {
  if (isPlanningWithUser(thread, pendingTasks)) return 'waiting'
  if (thread.taskCancelled && thread.status !== 'working') return 'stopped'
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
  items: ThreadItem[],
  pendingTasks: Record<string, true> = {}
): ThreadAgent[] {
  const tasks = threads
    .filter((thread) => thread.parentThreadId === threadId)
    .map(
      (thread): ThreadAgent => ({
        kind: 'task',
        id: thread.id,
        title: thread.title,
        state: taskState(thread, pendingTasks),
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

/** When a task's row places it: how long it has run, or when it ended. */
export type TaskTime =
  | { kind: 'elapsed'; milliseconds: number }
  | { kind: 'ended'; at: string }

export function taskTime(
  thread: Thread,
  state: AgentState,
  now: number
): TaskTime {
  if (state === 'running' || state === 'waiting') {
    const started = Date.parse(thread.createdAt)
    return {
      kind: 'elapsed',
      milliseconds: Number.isFinite(started) ? Math.max(0, now - started) : 0
    }
  }
  return { kind: 'ended', at: thread.updatedAt }
}

/**
 * The tasks a thread delegated that still run and whose answer it waits for, oldest
 * first.
 */
export function runningTasks(
  threadId: string,
  threads: Thread[],
  pendingTasks: Record<string, true>
): Thread[] {
  return threads
    .filter(
      (thread) =>
        thread.parentThreadId === threadId &&
        thread.status === 'working' &&
        !thread.taskCancelled &&
        pendingTasks[thread.id] === true
    )
    .sort((a, b) => a.createdAt.localeCompare(b.createdAt))
}

/** How many characters of a task's message its preview keeps; mirrors hoshi-core. */
export const TASK_PREVIEW_CHARS = 160

/** `text` on one line, its whitespace collapsed, cut short; mirrors `task_preview`. */
export function taskPreview(text: string) {
  const line = text.split(/\s+/).filter(Boolean).join(' ')
  const chars = Array.from(line)
  if (chars.length <= TASK_PREVIEW_CHARS) return line
  return `${chars
    .slice(0, TASK_PREVIEW_CHARS - 1)
    .join('')
    .trimEnd()}…`
}

/** A preview as plain text: links keep their label, markdown marks go. */
export function plainPreview(text: string) {
  return text
    .replace(/\[([^\]]+)\]\([^)]*\)/g, '$1')
    .replace(/\*\*|__|`/g, '')
    .replace(/^[\s#>*-]+/, '')
    .replace(/\s+/g, ' ')
    .trim()
}
