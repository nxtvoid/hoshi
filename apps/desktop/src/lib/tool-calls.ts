import type { ThreadItem } from '@/bindings/ThreadItem'
import type { StreamKind } from '@/bindings/StreamKind'
import type { ItemContent } from '@/bindings/ItemContent'
import { REQUEST_TOOLS } from './requests'

export type ToolCallContent = Extract<ItemContent, { kind: 'toolCall' }>
export type ReasoningContent = Extract<ItemContent, { kind: 'reasoning' }>
export type ToolCallItem = ThreadItem & { content: ToolCallContent }

/** An item as the transcript shows it; `streaming` while its text still arrives. */
export type TranscriptItem = ThreadItem & { streaming?: boolean }
export type WorkItem = TranscriptItem & {
  content: ToolCallContent | ReasoningContent
}

export type StreamingText = { kind: StreamKind; text: string }

export type TranscriptRow =
  | { kind: 'item'; item: TranscriptItem }
  | { kind: 'work'; id: string; entries: WorkItem[] }
  | {
      kind: 'fold'
      id: string
      rows: TranscriptRow[]
      /** From the user's message to the agent's answer; `null` when unknown. */
      durationMs: number | null
    }

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

export function isSubagentCall(item: TranscriptItem): item is ToolCallItem {
  return (
    item.content.kind === 'toolCall' && SUBAGENT_TOOLS.has(item.content.name)
  )
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
    !isSubagentCall(item)
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
    const last = rows.at(-1)
    if (!isWork(item)) rows.push({ kind: 'item', item })
    else if (last?.kind === 'work') last.entries.push(item)
    else rows.push({ kind: 'work', id: item.id, entries: [item] })
  }
  return rows
}

/**
 * A finished turn's work, between the user's message and the agent's final answer, folds
 * into one row. The running turn, and a turn that ended any other way, stay open.
 */
export function foldTurns(
  rows: TranscriptRow[],
  working: boolean
): TranscriptRow[] {
  const folded: TranscriptRow[] = []
  let asked: TranscriptItem | null = null
  let turn: TranscriptRow[] = []
  const close = (running: boolean) => {
    const answer = turn.at(-1)
    const work = turn.slice(0, -1)
    const foldable =
      !running &&
      asked !== null &&
      answer?.kind === 'item' &&
      answer.item.content.kind === 'assistantMessage' &&
      work.some((row) => row.kind === 'work')
    if (asked && answer?.kind === 'item' && foldable) {
      const duration =
        Date.parse(answer.item.createdAt) - Date.parse(asked.createdAt)
      folded.push(
        {
          kind: 'fold',
          id: `fold:${asked.id}`,
          rows: work,
          durationMs: Number.isFinite(duration) ? duration : null
        },
        answer
      )
    } else {
      folded.push(...turn)
    }
    turn = []
  }
  for (const row of rows) {
    if (row.kind === 'item' && row.item.content.kind === 'userMessage') {
      close(false)
      folded.push(row)
      asked = row.item
    } else {
      turn.push(row)
    }
  }
  close(working)
  return folded
}

/** The latest entry still going: a call without output, or reasoning still streaming. */
export function activeEntry(entries: WorkItem[]) {
  for (let index = entries.length - 1; index >= 0; index -= 1) {
    const entry = entries[index] as WorkItem
    if (entry.streaming) return entry
    if (entry.content.kind === 'toolCall' && entry.content.output === null) {
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
