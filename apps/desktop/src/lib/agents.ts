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

/**
 * Threads at rest that still have something going, by thread id: commands in the
 * background, or a task of their own in flight.
 */
export type BusyThreads = Record<string, unknown>

/**
 * Whether a task is still to answer the thread that delegated it: its answer hasn't gone
 * back yet, or it has and what the task left going will have it answer again.
 */
export function inFlight(
  thread: Thread,
  pendingTasks: Record<string, true>,
  busy: BusyThreads = {}
) {
  return (
    thread.parentThreadId !== null &&
    !thread.taskCancelled &&
    (pendingTasks[thread.id] === true || thread.id in busy)
  )
}

/**
 * The threads at rest that still have something going: those with commands
 * `inBackground`, and those a task in flight is still to answer, however deep.
 */
export function busyThreads(
  threads: Thread[],
  pendingTasks: Record<string, true>,
  inBackground: Record<string, unknown>
): Record<string, true> {
  const busy: Record<string, true> = {}
  for (const id of Object.keys(inBackground)) busy[id] = true
  let grew = true
  while (grew) {
    grew = false
    for (const thread of threads) {
      const parent = thread.parentThreadId
      if (parent && !busy[parent] && inFlight(thread, pendingTasks, busy)) {
        busy[parent] = true
        grew = true
      }
    }
  }
  return busy
}

/**
 * How a delegated task stands, read from its thread. At rest it still runs while it is in
 * flight: its answer is on its way, or it will answer again.
 */
export function taskState(
  thread: Thread,
  pendingTasks: Record<string, true>,
  busy: BusyThreads = {}
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
      return inFlight(thread, pendingTasks, busy) ? 'running' : 'done'
  }
}

/** How a subagent stands, read from the call that started it. */
export function subagentState(call: {
  output: string | null
  isError: boolean
  interrupted: boolean
}): AgentState {
  if (call.interrupted) return 'stopped'
  if (call.output === null) return 'running'
  return call.isError ? 'failed' : 'done'
}

/**
 * The thread's agents, newest first: the tasks it delegated, and the subagents its loaded
 * transcript shows.
 */
export function threadAgents(
  threadId: string,
  threads: Thread[],
  items: ThreadItem[],
  pendingTasks: Record<string, true> = {},
  busy: BusyThreads = {}
): ThreadAgent[] {
  const tasks = threads
    .filter((thread) => thread.parentThreadId === threadId)
    .map(
      (thread): ThreadAgent => ({
        kind: 'task',
        id: thread.id,
        title: thread.title,
        state: taskState(thread, pendingTasks, busy),
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
        state: content ? subagentState(content) : 'done',
        at: item.createdAt
      }
    })
  return [...tasks, ...subagents].sort((a, b) => b.at.localeCompare(a.at))
}

/** The tasks a thread delegated, and those they delegated in turn, which go with it. */
export function delegatedTasks(threadId: string, threads: Thread[]): Thread[] {
  return threads
    .filter((thread) => thread.parentThreadId === threadId)
    .flatMap((task) => [task, ...delegatedTasks(task.id, threads)])
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
 * The tasks a thread delegated that are in flight and run, oldest first. Those waiting for
 * the user are asked about elsewhere.
 */
export function runningTasks(
  threadId: string,
  threads: Thread[],
  pendingTasks: Record<string, true>,
  busy: BusyThreads = {}
): Thread[] {
  return threads
    .filter(
      (thread) =>
        thread.parentThreadId === threadId &&
        inFlight(thread, pendingTasks, busy) &&
        taskState(thread, pendingTasks, busy) === 'running'
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
