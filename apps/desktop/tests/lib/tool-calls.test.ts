import { describe, expect, test } from 'bun:test'
import type { ThreadItem } from '../../src/bindings/ThreadItem'
import {
  activeEntry,
  foldTurns,
  groupTranscript,
  reasoningPreview,
  relativePath,
  summarizeToolCalls,
  subagentTranscriptId,
  toolName,
  toolTarget,
  withStreaming,
  type ToolCallItem
} from '../../src/lib/tool-calls'

function tool(id: string, name: string, input: object = {}): ToolCallItem {
  return {
    id,
    threadId: 't',
    createdAt: '',
    content: {
      kind: 'toolCall',
      name,
      input: JSON.stringify(input),
      output: '',
      isError: false,
      detail: null,
      outputOmitted: false,
      interrupted: false
    }
  }
}

function text(id: string): ThreadItem {
  return {
    id,
    threadId: 't',
    createdAt: '',
    content: { kind: 'assistantMessage', text: id }
  }
}

function reasoning(id: string, value = id): ThreadItem {
  return {
    id,
    threadId: 't',
    createdAt: '',
    content: { kind: 'reasoning', text: value }
  }
}

const ids = (rows: ReturnType<typeof groupTranscript>) =>
  rows.map((row) =>
    row.kind === 'item' ? row.item.id : row.entries.map((entry) => entry.id)
  )

test('the calls a request stands for are left to the request', () => {
  const rows = groupTranscript([
    tool('a', 'AskUserQuestion'),
    tool('b', 'Read'),
    tool('c', 'ExitPlanMode'),
    tool('d', 'TaskUpdate')
  ])
  expect(rows).toEqual([
    { kind: 'work', id: 'b', entries: [tool('b', 'Read')] }
  ])
})

test('an MCP tool shows its own name and its server', () => {
  expect(toolName('mcp__claude_ai_Google_Drive__search_files')).toEqual({
    label: 'search files',
    source: 'Google Drive'
  })
  expect(toolName('Bash')).toEqual({ label: 'Bash', source: null })
})

test('a subagent stands apart from the work, its transcript beside the thread', () => {
  const rows = groupTranscript([
    tool('a', 'Read'),
    tool('b', 'Agent', { description: 'Count files' }),
    tool('c', 'Grep')
  ])

  expect(rows.map((row) => row.kind)).toEqual(['work', 'item', 'work'])
  expect(subagentTranscriptId('t', 'toolu_1')).toBe('t:toolu_1')
  expect(subagentTranscriptId('t:toolu_1', 'toolu_2')).toBe('t:toolu_2')
})

describe('groupTranscript', () => {
  test('folds consecutive tool calls into one row between messages', () => {
    const rows = groupTranscript([
      text('a'),
      tool('1', 'Bash'),
      tool('2', 'Write'),
      text('b'),
      tool('3', 'Read')
    ])
    expect(ids(rows)).toEqual(['a', ['1', '2'], 'b', ['3']])
  })

  test('reasoning between tool calls stays in their row', () => {
    const rows = groupTranscript([
      reasoning('r1'),
      tool('1', 'Bash'),
      reasoning('r2'),
      tool('2', 'Read'),
      text('a')
    ])
    expect(ids(rows)).toEqual([['r1', '1', 'r2', '2'], 'a'])
  })
})

describe('withStreaming', () => {
  test('streaming items follow the stored ones until they arrive whole', () => {
    const items = [text('a'), reasoning('r', 'done')]
    const merged = withStreaming(items, {
      r: { kind: 'reasoning', text: 'do' },
      b: { kind: 'text', text: 'partial' }
    })

    expect(merged.map((item) => item.id)).toEqual(['a', 'r', 'b'])
    expect(merged[1]?.streaming).toBeUndefined()
    expect(merged[2]).toMatchObject({
      streaming: true,
      content: { kind: 'assistantMessage', text: 'partial' }
    })
    expect(withStreaming(items, {})).toBe(items)
  })
})

describe('activeEntry', () => {
  test('is the latest call without output or reasoning still streaming', () => {
    const running: ToolCallItem = {
      ...tool('2', 'Bash'),
      content: { ...tool('2', 'Bash').content, output: null }
    }
    const entries = groupTranscript([tool('1', 'Read'), running])
    const [row] = entries

    expect(row?.kind === 'work' && activeEntry(row.entries)?.id).toBe('2')
    const streamed = withStreaming([tool('1', 'Read')], {
      r: { kind: 'reasoning', text: 'hm' }
    })
    const [work] = groupTranscript(streamed)
    expect(work?.kind === 'work' && activeEntry(work.entries)?.id).toBe('r')
    const [done] = groupTranscript([tool('1', 'Read'), reasoning('r')])
    expect(done?.kind === 'work' && activeEntry(done.entries)).toBeNull()
  })
})

describe('reasoningPreview', () => {
  test('shows the latest line while streaming and the first once done', () => {
    const value = '**Checking** the tests.\n\nThen `fix` the build.'

    expect(reasoningPreview(value, true)).toBe('Then fix the build.')
    expect(reasoningPreview(value, false)).toBe('Checking the tests.')
  })
})

describe('summarizeToolCalls', () => {
  test('counts edited files once each', () => {
    const calls = [
      tool('1', 'Bash'),
      tool('2', 'Write', { file_path: 'C:\\repo\\a.md' }),
      tool('3', 'Edit', { file_path: 'C:/repo/a.md' }),
      tool('4', 'Write', { file_path: 'C:\\repo\\b.md' })
    ]
    expect(summarizeToolCalls(calls)).toEqual([
      { action: 'command', count: 1 },
      { action: 'edit', count: 2 }
    ])
  })

  test('names the two most telling actions and counts the rest', () => {
    const calls = [
      tool('1', 'Read'),
      tool('2', 'Grep'),
      tool('3', 'Bash'),
      tool('4', 'TodoWrite'),
      tool('5', 'Edit', { file_path: 'a' })
    ]
    expect(summarizeToolCalls(calls)).toEqual([
      { action: 'command', count: 1 },
      { action: 'edit', count: 1 },
      { action: 'others', count: 3 }
    ])
  })
})

describe('toolTarget', () => {
  test('shows file paths relative to the working directory', () => {
    const input = JSON.stringify({ file_path: 'C:\\Repo\\src\\main.ts' })
    expect(toolTarget('Write', input, 'c:/repo')).toBe('src/main.ts')
  })

  test('keeps paths outside the working directory whole', () => {
    expect(relativePath('/home/me/.claude/MEMORY.md', '/home/me/repo')).toBe(
      '/home/me/.claude/MEMORY.md'
    )
  })

  test('shows the command of a shell call', () => {
    const input = JSON.stringify({ command: 'git status', description: 'x' })
    expect(toolTarget('Bash', input, null)).toBe('git status')
  })
})

describe('foldTurns', () => {
  const user = (id: string, createdAt: string): ThreadItem => ({
    id,
    threadId: 't',
    createdAt,
    content: { kind: 'userMessage', text: id, attachments: [], steer: false }
  })
  const answer = (id: string, createdAt: string): ThreadItem => ({
    ...text(id),
    createdAt
  })

  test('folds the work of a finished turn behind its answer', () => {
    const rows = groupTranscript([
      user('u1', '2026-11-08T10:00:00Z'),
      tool('r', 'Read'),
      text('note'),
      tool('e', 'Edit'),
      answer('a1', '2026-11-08T10:01:12Z')
    ])

    const folded = foldTurns(rows, false)

    expect(folded.map((row) => row.kind)).toEqual(['item', 'fold', 'item'])
    const fold = folded[1]
    expect(fold?.kind === 'fold' && fold.durationMs).toBe(72_000)
    expect(fold?.kind === 'fold' && fold.rows.length).toBe(3)
  })

  test('leaves the running turn, and turns without work or answer, open', () => {
    const rows = groupTranscript([
      user('u1', '2026-11-08T10:00:00Z'),
      answer('a1', '2026-11-08T10:00:05Z'),
      user('u2', '2026-11-08T10:01:00Z'),
      tool('r', 'Read'),
      user('u3', '2026-11-08T10:02:00Z'),
      tool('s', 'Grep'),
      answer('a3', '2026-11-08T10:02:30Z')
    ])

    expect(foldTurns(rows, false).map((row) => row.kind)).toEqual([
      'item',
      'item',
      'item',
      'work',
      'item',
      'fold',
      'item'
    ])
    expect(foldTurns(rows, true).at(-2)?.kind).toBe('work')
  })

  test('keeps live rows out of the fold, after it', () => {
    const rows = groupTranscript([
      user('u1', '2026-11-08T10:00:00Z'),
      tool('r', 'Read'),
      tool('agent', 'Agent'),
      tool('e', 'Edit'),
      answer('a1', '2026-11-08T10:01:00Z')
    ])

    const folded = foldTurns(
      rows,
      false,
      (row) => row.kind === 'item' && row.item.id === 'agent'
    )

    expect(
      folded.map((row) => (row.kind === 'item' ? row.item.id : row.kind))
    ).toEqual(['u1', 'fold', 'agent', 'a1'])
    const fold = folded[1]
    expect(fold?.kind === 'fold' && fold.rows.length).toBe(2)
  })
})
