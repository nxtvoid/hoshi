import {
  delegationOf,
  isDelegation,
  type TaskOutcome,
  type TranscriptItem,
  type TranscriptRow
} from './tool-calls'

/** How a task's answer says it ended, from the line hoshi puts first. */
export function taskOutcome(text: string): TaskOutcome {
  const head = text.split('\n', 1)[0] ?? ''
  if (head.endsWith('[failed]:')) return 'failed'
  if (head.endsWith('[stopped]:')) return 'stopped'
  return 'completed'
}

/** A message the user sent, which starts what the agent works on next. */
function startsWork(row: TranscriptRow) {
  if (row.kind !== 'item') return false
  const { content } = row.item
  return content.kind === 'userMessage' && !content.steer && !content.taskResult
}

function taskResultOf(row: TranscriptRow) {
  if (row.kind !== 'item') return null
  const { content } = row.item
  return content.kind === 'userMessage' ? content.taskResult : null
}

function isAnswer(row: TranscriptRow) {
  return row.kind === 'item' && row.item.content.kind === 'assistantMessage'
}

function delegates(row: TranscriptRow) {
  return row.kind === 'item' && isDelegation(row.item)
}

function flatten(rows: TranscriptRow[]): TranscriptRow[] {
  return rows.flatMap((row) => (row.kind === 'fold' ? flatten(row.rows) : row))
}

function timeOf(item: TranscriptItem) {
  const time = Date.parse(item.createdAt)
  return Number.isFinite(time) ? time : null
}

function rowTime(row: TranscriptRow | undefined) {
  return row?.kind === 'item' ? timeOf(row.item) : null
}

/** The tasks a row delegates or hears back from. */
function tasksOf(row: TranscriptRow): string[] {
  if (row.kind === 'fold') return row.rows.flatMap(tasksOf)
  const result = taskResultOf(row)
  if (result) return [result]
  if (row.kind !== 'item' || !isDelegation(row.item)) return []
  const { taskId } = delegationOf(row.item.content)
  return taskId ? [taskId] : []
}

/**
 * Between two messages of the user, the work on the tasks the agent delegated. The tasks'
 * answers stay out of view, as each task's row shows its own, and the agent's replies to
 * them read as notes, the work behind each reply left out.
 *
 * What is settled folds behind one line: everything up to the last answer that left none
 * of the tasks so far `active`. What follows stays in view: the agent's last reply since,
 * the work still going on toward it, and the tasks it delegates next. What folded never
 * comes back into view, however the agent goes on.
 */
export function foldTaskRuns(
  rows: TranscriptRow[],
  active: (taskId: string) => boolean
): TranscriptRow[] {
  const folded: TranscriptRow[] = []
  let start: TranscriptItem | null = null
  let part: TranscriptRow[] = []
  const close = () => {
    folded.push(...taskRuns(start, part, active))
    part = []
  }
  for (const row of rows) {
    if (startsWork(row)) {
      close()
      folded.push(row)
      start = row.kind === 'item' ? row.item : null
    } else {
      part.push(row)
    }
  }
  close()
  return folded
}

function taskRuns(
  start: TranscriptItem | null,
  part: TranscriptRow[],
  active: (taskId: string) => boolean
): TranscriptRow[] {
  if (!part.some((row) => taskResultOf(row) !== null)) return part
  const tasks: string[] = []
  let settled = -1
  part.forEach((row, index) => {
    tasks.push(...tasksOf(row))
    if (taskResultOf(row) !== null && !tasks.some(active)) settled = index
  })
  const cut = settled === -1 ? 0 : replyAfter(part, settled)
  const toUser = new Set<TranscriptRow>()
  const before = withNotes(part.slice(0, cut), false, toUser)
  const after = withNotes(part.slice(cut), true)
  if (!start || cut === 0) return [...before, ...after]
  const ended = rowTime(part[cut]) ?? rowTime(part[settled])
  return [...folded(start, before, toUser, ended), ...after]
}

/**
 * The settled rows behind fold lines. What the user said along the way and the agent's
 * answers to them stay in view, as they do when a turn folds, with a line of their own
 * for what came before and after each.
 */
function folded(
  start: TranscriptItem,
  rows: TranscriptRow[],
  toUser: Set<TranscriptRow>,
  ended: number | null
): TranscriptRow[] {
  const shown: TranscriptRow[] = []
  let since = start
  let behind: TranscriptRow[] = []
  const fold = (until: number | null) => {
    if (behind.length === 0) return
    const began = timeOf(since)
    shown.push({
      kind: 'fold',
      id: `tasks:${since.id}`,
      rows: flatten(behind),
      ending: 'answered',
      after: since === start ? 'start' : 'steer',
      durationMs: began !== null && until !== null ? until - began : null
    })
    behind = []
  }
  for (const row of rows) {
    if (row.kind === 'item' && toUser.has(row)) {
      fold(timeOf(row.item))
      shown.push(row)
      since = row.item
    } else {
      behind.push(row)
    }
  }
  fold(ended)
  return shown
}

/**
 * Where what stays in view begins, after the answer at `settled`: the agent's last reply
 * before it delegates again, else the task it delegates next, else what it works on.
 */
function replyAfter(part: TranscriptRow[], settled: number) {
  let reply = -1
  for (let index = settled + 1; index < part.length; index += 1) {
    const row = part[index] as TranscriptRow
    if (delegates(row)) return reply === -1 ? index : reply
    if (isAnswer(row)) reply = index
  }
  return reply === -1 ? settled + 1 : reply
}

/**
 * The rows without the tasks' answers, the agent's replies to them as notes. The agent
 * answers what reaches it in the order it came, so each reply goes with the oldest answer
 * or message of the user still without one; `toUser` gets those messages and the replies
 * to them. While the tasks are `inView`, the work behind each reply is left out too.
 */
function withNotes(
  rows: TranscriptRow[],
  inView: boolean,
  toUser?: Set<TranscriptRow>
): TranscriptRow[] {
  const unanswered: (TaskOutcome | 'user')[] = []
  let replyTo: TaskOutcome | 'user' | null = null
  let answered = false
  return rows.flatMap((row): TranscriptRow[] => {
    if (row.kind !== 'item') {
      return row.kind === 'fold' && inView && answered ? [] : [row]
    }
    const { content } = row.item
    if (content.kind === 'userMessage') {
      unanswered.push(content.taskResult ? taskOutcome(content.text) : 'user')
      answered ||= content.taskResult !== null
      if (content.taskResult) return []
      toUser?.add(row)
      return [row]
    }
    if (!isAnswer(row)) return [row]
    replyTo = unanswered.shift() ?? replyTo
    if (replyTo === 'user') toUser?.add(row)
    return replyTo === null || replyTo === 'user'
      ? [row]
      : [{ ...row, note: replyTo }]
  })
}
