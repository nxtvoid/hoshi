import type { DiffFile } from '@/bindings/DiffFile'
import type { ThreadItem } from '@/bindings/ThreadItem'
import type { StreamKind } from '@/bindings/StreamKind'
import type { ItemContent } from '@/bindings/ItemContent'
import { REQUEST_TOOLS } from './requests'
import { HTML_RENDER_TOOL } from './html-page'

export type ToolCallContent = Extract<ItemContent, { kind: 'toolCall' }>
export type ReasoningContent = Extract<ItemContent, { kind: 'reasoning' }>
export type ToolCallItem = ThreadItem & { content: ToolCallContent }

/** An item as the transcript shows it; `streaming` while its text still arrives. */
export type TranscriptItem = ThreadItem & { streaming?: boolean }
export type WorkItem = TranscriptItem & {
  content: ToolCallContent | ReasoningContent
}

export type StreamingText = { kind: StreamKind; text: string }

/** The files a finished turn changed, shown under its last answer. */
export type TurnChanges = { turnId: string; files: DiffFile[] }

/** How a delegated task ended, as its answer says. */
export type TaskOutcome = 'completed' | 'failed' | 'stopped'

export type TranscriptRow =
  | {
      kind: 'item'
      item: TranscriptItem
      changes?: TurnChanges
      /** The agent's reply to a task's answer, while the work on its tasks goes on. */
      note?: TaskOutcome
    }
  | { kind: 'work'; id: string; entries: WorkItem[] }
  | {
      kind: 'fold'
      id: string
      rows: TranscriptRow[]
      /** How the work it holds ended: answered, cut by the user's steer, or stopped. */
      ending: FoldEnding
      /** What the work follows: a steer, or anything else that started it. */
      after: 'start' | 'steer'
      /** From what started the work to its end; `null` when unknown. */
      durationMs: number | null
    }

export type FoldEnding = 'answered' | 'steered' | 'stopped'

export type ToolAction =
  | 'command'
  | 'edit'
  | 'read'
  | 'search'
  | 'web'
  | 'other'

const ACTIONS: Record<string, ToolAction> = {
  Bash: 'command',
  PowerShell: 'command',
  BashOutput: 'command',
  Edit: 'edit',
  MultiEdit: 'edit',
  Write: 'edit',
  NotebookEdit: 'edit',
  Read: 'read',
  Grep: 'search',
  Glob: 'search',
  WebFetch: 'web',
  WebSearch: 'web'
}

export function toolAction(name: string): ToolAction {
  return ACTIONS[name] ?? 'other'
}

/** The name to show for a tool: an MCP tool's own name, with the server it comes from. */
export function toolName(name: string): {
  label: string
  source: string | null
} {
  const mcp = /^mcp__(.+?)__(.+)$/.exec(name)
  if (!mcp) return { label: name, source: null }
  const words = (text: string) =>
    text
      .replace(/^claude_ai_/, '')
      .replaceAll('_', ' ')
      .trim()
  return { label: words(mcp[2] ?? ''), source: words(mcp[1] ?? '') }
}

/** Tools that start a subagent, shown as its own card rather than among the work. */
const SUBAGENT_TOOLS = new Set(['Agent', 'Task'])

/** The tool an agent delegates a task to a child thread with. */
const DELEGATE_TOOL = 'mcp__hoshi__delegate_task'

export function isDelegation(item: TranscriptItem): item is ToolCallItem {
  return item.content.kind === 'toolCall' && item.content.name === DELEGATE_TOOL
}

/** What a delegation names: its title, the child thread once started, or why it failed. */
export function delegationOf(call: ToolCallContent): {
  title: string
  taskId: string | null
  error: string | null
} {
  const input = parseInput(call.input)
  const output = call.output && !call.isError ? parseInput(call.output) : null
  const text = (value: Record<string, unknown> | null, key: string) =>
    typeof value?.[key] === 'string' ? (value[key] as string) : null
  return {
    title:
      text(output, 'title') ??
      text(input, 'title') ??
      text(input, 'task') ??
      '',
    taskId: text(output, 'taskId'),
    error: call.isError ? call.output : null
  }
}

/** A page the agent showed: its own row once it is kept, a tool call until then. */
export function isHtmlRender(item: TranscriptItem): item is ToolCallItem {
  return (
    item.content.kind === 'toolCall' &&
    item.content.name === HTML_RENDER_TOOL &&
    item.content.output !== null &&
    !item.content.isError
  )
}

export function isSubagentCall(item: TranscriptItem): item is ToolCallItem {
  return (
    item.content.kind === 'toolCall' && SUBAGENT_TOOLS.has(item.content.name)
  )
}

/**
 * Whether a row stays in view when its turn folds: a delegated task, which runs on its
 * own, a page the agent showed, or a subagent without its answer.
 */
export function staysInView(row: TranscriptRow) {
  if (row.kind !== 'item' || row.item.content.kind !== 'toolCall') return false
  if (isDelegation(row.item) || isHtmlRender(row.item)) return true
  const call = row.item.content
  return isSubagentCall(row.item) && call.output === null && !call.interrupted
}

/** Where a subagent's transcript is kept; mirrors `subagent_thread_id` in hoshi-core. */
export function subagentTranscriptId(threadId: string, toolUseId: string) {
  return `${threadId.split(':')[0]}:${toolUseId}`
}

/** What the agent asked a subagent, read from the call that started it. */
export function subagentInput(input: string) {
  const value = parseInput(input)
  const text = (key: string) =>
    typeof value?.[key] === 'string' ? (value[key] as string) : ''
  return {
    description: text('description'),
    kind: text('subagent_type'),
    prompt: text('prompt')
  }
}

function isWork(item: TranscriptItem): item is WorkItem {
  return (
    (item.content.kind === 'toolCall' || item.content.kind === 'reasoning') &&
    !isSubagentCall(item) &&
    !isDelegation(item) &&
    !isHtmlRender(item)
  )
}

export function isToolCall(item: TranscriptItem): item is ToolCallItem {
  return item.content.kind === 'toolCall'
}

/**
 * The stored items followed by those still streaming, so a streamed item keeps its place
 * when it arrives whole.
 */
export function withStreaming(
  items: ThreadItem[],
  streaming: Record<string, StreamingText>
): TranscriptItem[] {
  const ids = Object.keys(streaming).filter(
    (id) => streaming[id]?.kind !== 'toolOutput'
  )
  if (ids.length === 0) return items
  const stored = new Set(items.map((item) => item.id))
  const pending = ids.filter((id) => !stored.has(id))
  if (pending.length === 0) return items
  return [
    ...items,
    ...pending.map((id): TranscriptItem => {
      const { kind, text } = streaming[id] as StreamingText
      return {
        id,
        threadId: '',
        createdAt: '',
        streaming: true,
        content:
          kind === 'reasoning'
            ? { kind: 'reasoning', text }
            : { kind: 'assistantMessage', text }
      }
    })
  ]
}

/**
 * Runs of consecutive tool calls and reasoning become one row, so a busy turn reads as
 * one step.
 */
export function groupTranscript(items: TranscriptItem[]): TranscriptRow[] {
  const rows: TranscriptRow[] = []
  for (const item of items) {
    if (isToolCall(item) && REQUEST_TOOLS.has(item.content.name)) continue
    const { content } = item
    if (content.kind === 'approval' && content.toolCallId !== null) continue
    if (content.kind === 'turnChanges') {
      if (content.files?.length) {
        const turnId = item.id.slice(0, -CHANGES_SUFFIX.length)
        attachToAnswer(rows, { turnId, files: content.files })
      }
      continue
    }
    const last = rows.at(-1)
    if (!isWork(item)) rows.push({ kind: 'item', item })
    else if (last?.kind === 'work') last.entries.push(item)
    else rows.push({ kind: 'work', id: item.id, entries: [item] })
  }
  return rows
}

const CHANGES_SUFFIX = ':changes'

/** Puts a turn's changes on its last answer; a turn that never answered shows none. */
function attachToAnswer(rows: TranscriptRow[], changes: TurnChanges) {
  for (let index = rows.length - 1; index >= 0; index -= 1) {
    const row = rows[index] as TranscriptRow
    if (row.kind !== 'item') continue
    const { content } = row.item
    if (content.kind === 'userMessage' && !content.steer) return
    if (content.kind === 'assistantMessage') {
      rows[index] = { ...row, changes }
      return
    }
  }
}

function isAgent(row: TranscriptRow) {
  return (
    row.kind === 'item' && (isSubagentCall(row.item) || isDelegation(row.item))
  )
}

/**
 * The work of each finished run folds into one row, as t3code does: between what started
 * it (the user's message, a background task that ended, or the run before) and the agent's
 * answer, which stays in view, with the finished work after it. A steer cuts the run: what
 * came before folds whole, and what follows folds on its own. A stopped run folds whole.
 * The running turn, and a failed one, stay open, as do rows still `live`, after the fold.
 * A run that a steer ended before any answer does not cut the turn, and while working
 * neither does any run that ended since the latest response started.
 */
export function foldTurns(
  rows: TranscriptRow[],
  working: boolean,
  live: (row: TranscriptRow) => boolean = () => false
): TranscriptRow[] {
  const folded: TranscriptRow[] = []
  let started: TranscriptItem | null = null
  let after: 'start' | 'steer' = 'start'
  let turn: TranscriptRow[] = []
  const hasWork = (part: TranscriptRow[]) =>
    part.some((row) => !live(row) && (row.kind === 'work' || isAgent(row)))
  const fold = (
    part: TranscriptRow[],
    ending: FoldEnding,
    endedAt: string
  ): TranscriptRow[] => {
    if (!started) return part
    const duration = Date.parse(endedAt) - Date.parse(started.createdAt)
    return [
      {
        kind: 'fold',
        id: `fold:${started.id}`,
        rows: part.filter((row) => !live(row)),
        ending,
        after,
        durationMs: Number.isFinite(duration) ? duration : null
      },
      ...part.filter(live)
    ]
  }
  const settle = (
    part: TranscriptRow[],
    steer: TranscriptItem | null
  ): TranscriptRow[] => {
    if (
      !started ||
      !hasWork(part) ||
      part.some(
        (row) => row.kind === 'item' && row.item.content.kind === 'turnFailed'
      )
    ) {
      return part
    }
    if (steer) return fold(part, 'steered', steer.createdAt)
    const last = part.at(-1)
    if (last?.kind === 'item' && last.item.content.kind === 'turnInterrupted') {
      const before = part.slice(0, -1)
      const said = before.at(-1)
      const answer =
        said?.kind === 'item' && said.item.content.kind === 'assistantMessage'
          ? said
          : null
      const work = answer ? before.slice(0, -1) : before
      if (!hasWork(work)) return part
      return [
        ...fold(work, 'stopped', last.item.createdAt),
        ...(answer ? [answer] : [])
      ]
    }
    let answerAt = part.length - 1
    while (answerAt >= 0) {
      const row = part[answerAt] as TranscriptRow
      if (row.kind === 'item' && row.item.content.kind === 'assistantMessage') {
        break
      }
      answerAt -= 1
    }
    const answer = part[answerAt]
    if (answer?.kind !== 'item') return part
    const trailing = part.slice(answerAt + 1)
    const settled = (row: TranscriptRow) =>
      row.kind === 'work' &&
      !live(row) &&
      !row.entries.some(
        (entry) => entry.content.kind === 'toolCall' && entry.content.isError
      )
    const work = [...part.slice(0, answerAt), ...trailing.filter(settled)]
    if (!hasWork(work)) return part
    return [
      ...fold(work, 'answered', answer.item.createdAt),
      answer,
      ...trailing.filter((row) => !settled(row))
    ]
  }
  /** Parts of the turn a steer cut, folded only once the turn is over. */
  let held: { open: TranscriptRow[]; settled: TranscriptRow[] }[] = []
  let turnStart: number | null = null
  const starts = responseStarts(rows)
  const responseAt = working ? Math.max(-1, ...starts) : rows.length
  const close = (running: boolean) => {
    const end = endOf(turn)
    const wholeTurn = (row: TranscriptRow): TranscriptRow =>
      row.kind === 'fold' &&
      row.after === 'start' &&
      turnStart !== null &&
      end !== null
        ? { ...row, durationMs: end - turnStart }
        : row
    for (const part of held)
      folded.push(...(running ? part.open : part.settled.map(wholeTurn)))
    held = []
    folded.push(...(running ? turn : settle(turn, null)))
    turn = []
  }
  for (const [index, row] of rows.entries()) {
    const content = row.kind === 'item' ? row.item.content : null
    if (row.kind !== 'item' || !content) {
      turn.push(row)
    } else if (content.kind === 'userMessage' && content.steer) {
      if (after === 'start') turnStart = started && timeOf(started)
      held.push({
        open: [...turn, row],
        settled: [...settle(turn, row.item), row]
      })
      turn = []
      started = row.item
      after = 'steer'
    } else if (starts.has(index)) {
      close(false)
      folded.push(row)
      started = row.item
      after = 'start'
    } else if (content.kind === 'runEnded') {
      if (index > responseAt || (after === 'steer' && !answered(turn))) {
        continue
      }
      close(false)
      started = row.item
      after = 'start'
    } else {
      turn.push(row)
    }
  }
  close(working)
  return folded
}

/** Splits the id of a background task's end into its turn and the task; mirrors hoshi-core. */
const TASK_ENDED = ':ended:'

/**
 * Where each response starts: at a message from the user, or at a background task that
 * ended and gets a run of its own. A task that ended while a run of its own turn went on
 * is taken in by that run and starts nothing.
 */
function responseStarts(rows: TranscriptRow[]) {
  const starts = new Set<number>()
  let turn: string | null = null
  let betweenRuns = false
  for (const [index, row] of rows.entries()) {
    const content = row.kind === 'item' ? row.item.content : null
    if (row.kind !== 'item' || !content) {
      betweenRuns = false
    } else if (content.kind === 'userMessage') {
      if (content.steer) continue
      starts.add(index)
      turn = row.item.id
      betweenRuns = false
    } else if (content.kind === 'backgroundTaskEnded') {
      const at = row.item.id.indexOf(TASK_ENDED)
      const endedIn = at < 0 ? null : row.item.id.slice(0, at)
      if (endedIn !== null && endedIn === turn && !betweenRuns) continue
      starts.add(index)
      turn = endedIn
      betweenRuns = false
    } else if (content.kind === 'runEnded') {
      betweenRuns = true
    } else if (
      content.kind === 'turnInterrupted' ||
      content.kind === 'turnFailed'
    ) {
      turn = null
    } else {
      betweenRuns = false
    }
  }
  return starts
}

function timeOf(item: TranscriptItem) {
  const time = Date.parse(item.createdAt)
  return Number.isFinite(time) ? time : null
}

/** When the last row of a part happened, or `null` when unknown. */
function endOf(part: TranscriptRow[]) {
  for (let index = part.length - 1; index >= 0; index -= 1) {
    const row = part[index] as TranscriptRow
    const item =
      row.kind === 'item'
        ? row.item
        : row.kind === 'work'
          ? row.entries.at(-1)
          : null
    const time = item ? timeOf(item) : null
    if (time !== null) return time
  }
  return null
}

function answered(part: TranscriptRow[]) {
  return part.some(
    (row) => row.kind === 'item' && row.item.content.kind === 'assistantMessage'
  )
}

/** The latest entry still going: a call without output, or reasoning still streaming. */
export function activeEntry(entries: WorkItem[]) {
  for (let index = entries.length - 1; index >= 0; index -= 1) {
    const entry = entries[index] as WorkItem
    if (entry.streaming) return entry
    if (
      entry.content.kind === 'toolCall' &&
      entry.content.output === null &&
      !entry.content.interrupted
    ) {
      return entry
    }
  }
  return null
}

/**
 * One line of reasoning to show while it is folded: the latest while it streams, the
 * first once it is done, without markdown marks.
 */
export function reasoningPreview(text: string, streaming: boolean) {
  const lines = text
    .split(/\r?\n/)
    .map((line) => line.replace(/[*_`#>]/g, '').trim())
    .filter(Boolean)
  return (streaming ? lines.at(-1) : lines[0]) ?? ''
}

function parseInput(input: string): Record<string, unknown> | null {
  try {
    const value: unknown = JSON.parse(input)
    return value !== null && typeof value === 'object'
      ? (value as Record<string, unknown>)
      : null
  } catch {
    return null
  }
}

const PATH_KEYS = ['file_path', 'notebook_path', 'path']
const TARGET_KEYS = ['command', ...PATH_KEYS, 'pattern', 'url', 'query']

function filePath(input: string) {
  const value = parseInput(input)
  for (const key of PATH_KEYS) {
    if (typeof value?.[key] === 'string') return value[key]
  }
  return null
}

/** The one argument that says what a call did, with paths made relative to `cwd`. */
export function toolTarget(name: string, input: string, cwd: string | null) {
  const value = parseInput(input)
  if (!value) return input
  for (const key of TARGET_KEYS) {
    const target = value[key]
    if (typeof target !== 'string') continue
    return PATH_KEYS.includes(key) ? relativePath(target, cwd) : target
  }
  return toolAction(name) === 'other' ? '' : input
}

export function relativePath(path: string, cwd: string | null) {
  if (!cwd) return path
  const normalize = (value: string) =>
    value.replaceAll('\\', '/').replace(/\/+$/, '')
  const target = normalize(path)
  const root = normalize(cwd)
  const windows = /^[a-z]:\//i.test(root)
  const compare = (value: string) => (windows ? value.toLowerCase() : value)
  if (compare(target) === compare(root)) return '.'
  if (compare(target).startsWith(`${compare(root)}/`)) {
    return target.slice(root.length + 1)
  }
  return path
}

export type ToolSummaryPart = { action: ToolAction | 'others'; count: number }

const MAX_SUMMARY_ACTIONS = 2
const PRIORITY: Record<ToolAction, number> = {
  command: 0,
  edit: 0,
  read: 1,
  search: 1,
  web: 1,
  other: 2
}

/**
 * What a run of calls did, as counts per action in the order they first ran.
 * The two most telling actions are named; the rest are counted together.
 */
export function summarizeToolCalls(calls: ToolCallItem[]): ToolSummaryPart[] {
  const groups = new Map<ToolAction, ToolCallItem[]>()
  for (const call of calls) {
    const action = toolAction(call.content.name)
    groups.set(action, [...(groups.get(action) ?? []), call])
  }
  const parts = [...groups].map(([action, group], index) => ({
    action,
    index,
    count: action === 'edit' ? editedFileCount(group) : group.length,
    calls: group.length
  }))
  const named = [...parts]
    .sort(
      (a, b) => PRIORITY[a.action] - PRIORITY[b.action] || a.index - b.index
    )
    .slice(0, MAX_SUMMARY_ACTIONS)
    .sort((a, b) => a.index - b.index)
  const others =
    calls.length - named.reduce((total, part) => total + part.calls, 0)
  const summary: ToolSummaryPart[] = named.map(({ action, count }) => ({
    action,
    count
  }))
  if (others > 0) summary.push({ action: 'others', count: others })
  return summary
}

function editedFileCount(calls: ToolCallItem[]) {
  const files = new Set<string>()
  let unnamed = 0
  for (const call of calls) {
    const path = filePath(call.content.input)
    if (path) files.add(path.replaceAll('\\', '/'))
    else unnamed += 1
  }
  return files.size + unnamed
}
