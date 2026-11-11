import { describe, expect, test } from 'bun:test'
import type { ThreadItem } from '../../src/bindings/ThreadItem'
import {
  activeEntry,
  foldTurns,
  groupTranscript,
  readingChanges,
  reasoningPreview,
  relativePath,
  staysInView,
  statusAnchor,
  summarizeToolCalls,
  subagentTranscriptId,
  toolName,
  toolTarget,
  withStreaming,
  streamingShape,
  fromStreamingShape,
  type ToolCallItem,
  type TranscriptRow
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

  test('folds each run of a turn that went on behind its own answer', () => {
    const runEnded = (id: string, createdAt: string): ThreadItem => ({
      id,
      threadId: 't',
      createdAt,
      content: { kind: 'runEnded' }
    })
    const rows = groupTranscript([
      user('u1', '2026-11-08T10:00:00Z'),
      tool('agent', 'Read'),
      answer('a1', '2026-11-08T10:00:10Z'),
      runEnded('run1', '2026-11-08T10:00:11Z'),
      tool('r', 'Grep'),
      answer('a2', '2026-11-08T10:00:31Z'),
      runEnded('run2', '2026-11-08T10:00:32Z'),
      answer('a3', '2026-11-08T10:00:40Z')
    ])

    const folded = foldTurns(rows, false)

    expect(
      folded.map((row) => (row.kind === 'item' ? row.item.id : row.kind))
    ).toEqual(['u1', 'fold', 'a1', 'fold', 'a2', 'a3'])
    const second = folded[3]
    expect(second?.kind === 'fold' && second.durationMs).toBe(20_000)
  })

  test('folds a turn whose only work was agents, once they are done', () => {
    const rows = groupTranscript([
      user('u1', '2026-11-08T10:00:00Z'),
      tool('one', 'Agent'),
      tool('two', 'Agent'),
      answer('a1', '2026-11-08T10:00:10Z')
    ])
    const kinds = (folded: TranscriptRow[]) =>
      folded.map((row) => (row.kind === 'item' ? row.item.id : row.kind))

    expect(kinds(foldTurns(rows, false))).toEqual(['u1', 'fold', 'a1'])
    expect(kinds(foldTurns(rows, false, (row) => row.kind === 'item'))).toEqual(
      ['u1', 'one', 'two', 'a1']
    )
  })

  test('keeps a delegated task in view after its turn folds', () => {
    const rows = groupTranscript([
      user('u1', '2026-11-08T10:00:00Z'),
      tool('r', 'Read'),
      tool('task', 'mcp__hoshi__delegate_task', { title: 'Count' }),
      tool('done', 'Agent'),
      answer('a1', '2026-11-08T10:00:10Z')
    ])
    const kinds = (folded: TranscriptRow[]) =>
      folded.map((row) => (row.kind === 'item' ? row.item.id : row.kind))

    expect(kinds(foldTurns(rows, false, staysInView))).toEqual([
      'u1',
      'fold',
      'task',
      'a1'
    ])
  })

  const marker = (
    id: string,
    createdAt: string,
    content: ThreadItem['content']
  ): ThreadItem => ({ id, threadId: 't', createdAt, content })
  const shape = (folded: TranscriptRow[]) =>
    folded.map((row) =>
      row.kind === 'item'
        ? row.item.id
        : row.kind === 'fold'
          ? `${row.ending}:${row.after}:${row.rows.length}`
          : row.kind
    )

  test('starts a run at a background task that ended, which stays in view', () => {
    const rows = groupTranscript([
      user('u1', '2026-11-08T10:00:00Z'),
      tool('r', 'Read'),
      answer('a1', '2026-11-08T10:00:10Z'),
      marker('run1', '2026-11-08T10:00:11Z', { kind: 'runEnded' }),
      marker('ended', '2026-11-08T10:00:20Z', {
        kind: 'backgroundTaskEnded',
        toolCallId: 'agent',
        failed: false
      }),
      tool('g', 'Grep'),
      answer('a2', '2026-11-08T10:00:30Z')
    ])

    const folded = foldTurns(rows, false)

    expect(shape(folded)).toEqual([
      'u1',
      'answered:start:1',
      'a1',
      'ended',
      'answered:start:1',
      'a2'
    ])
    const second = folded[4]
    expect(second?.kind === 'fold' && second.durationMs).toBe(10_000)
  })

  test('a background task that ended while its turn still ran folds with that turn', () => {
    const ended = (id: string, createdAt: string) =>
      marker(id, createdAt, {
        kind: 'backgroundTaskEnded',
        toolCallId: 'dev',
        failed: true
      })
    const items = [
      user('u1', '2026-11-08T10:00:00Z'),
      tool('dev', 'Bash'),
      tool('install', 'Bash'),
      ended('u1:ended:b1', '2026-11-08T10:00:12Z'),
      tool('dev2', 'Bash'),
      answer('a1', '2026-11-08T10:00:28Z')
    ]

    const folded = foldTurns(groupTranscript(items), false)

    expect(shape(folded)).toEqual(['u1', 'answered:start:3', 'a1'])
    const fold = folded[1]
    expect(fold?.kind === 'fold' && fold.durationMs).toBe(28_000)
    expect(shape(foldTurns(groupTranscript(items.slice(0, 5)), true))).toEqual([
      'u1',
      'work',
      'u1:ended:b1',
      'work'
    ])
  })

  test('a background task that ended after a run of its turn starts the next run', () => {
    const rows = groupTranscript([
      user('u1', '2026-11-08T10:00:00Z'),
      tool('r', 'Read'),
      answer('a1', '2026-11-08T10:00:10Z'),
      marker('u1:run1', '2026-11-08T10:00:11Z', { kind: 'runEnded' }),
      marker('u1:ended:b1', '2026-11-08T10:00:20Z', {
        kind: 'backgroundTaskEnded',
        toolCallId: 'dev',
        failed: false
      }),
      tool('g', 'Grep'),
      answer('a2', '2026-11-08T10:00:30Z'),
      marker('woke:ended:b2', '2026-11-08T10:05:00Z', {
        kind: 'backgroundTaskEnded',
        toolCallId: 'dev',
        failed: false
      }),
      tool('h', 'Grep'),
      answer('a3', '2026-11-08T10:05:08Z')
    ])

    expect(shape(foldTurns(rows, false))).toEqual([
      'u1',
      'answered:start:1',
      'a1',
      'u1:ended:b1',
      'answered:start:1',
      'a2',
      'woke:ended:b2',
      'answered:start:1',
      'a3'
    ])
  })

  test('folds what a steer cut once the turn is over, and what followed apart', () => {
    const steer = marker('s', '2026-11-08T10:00:20Z', {
      kind: 'userMessage',
      text: 's',
      attachments: [],
      steer: true,
      taskResult: null
    })
    const items = [
      user('u1', '2026-11-08T10:00:00Z'),
      tool('r', 'Read'),
      text('partial'),
      steer,
      tool('e', 'Edit'),
      answer('a1', '2026-11-08T10:00:50Z')
    ]

    const settled = foldTurns(groupTranscript(items), false)
    expect(shape(settled)).toEqual([
      'u1',
      'steered:start:2',
      's',
      'answered:steer:1',
      'a1'
    ])
    const durations = settled.map((row) =>
      row.kind === 'fold' ? row.durationMs : null
    )
    expect(durations).toEqual([null, 50_000, null, 30_000, null])
    expect(shape(foldTurns(groupTranscript(items.slice(0, 5)), true))).toEqual([
      'u1',
      'work',
      'partial',
      's',
      'work'
    ])
  })

  test('a run a steer ended folds nothing while the turn still works', () => {
    const steer = marker('s', '2026-11-08T10:00:20Z', {
      kind: 'userMessage',
      text: 's',
      attachments: [],
      steer: true,
      taskResult: null
    })
    const items = [
      user('u1', '2026-11-08T10:00:00Z'),
      tool('r', 'Read'),
      steer,
      marker('run1', '2026-11-08T10:00:21Z', { kind: 'runEnded' }),
      tool('e', 'Edit')
    ]

    expect(shape(foldTurns(groupTranscript(items), true))).toEqual([
      'u1',
      'work',
      's',
      'work'
    ])
    expect(
      shape(
        foldTurns(
          groupTranscript([...items, answer('a1', '2026-11-08T10:00:50Z')]),
          false
        )
      )
    ).toEqual(['u1', 'steered:start:1', 's', 'answered:steer:1', 'a1'])
  })

  test('a run that ended before the latest response still folds while working', () => {
    const rows = groupTranscript([
      user('u1', '2026-11-08T10:00:00Z'),
      tool('r', 'Read'),
      answer('a1', '2026-11-08T10:00:10Z'),
      marker('run1', '2026-11-08T10:00:11Z', { kind: 'runEnded' }),
      marker('ended', '2026-11-08T10:00:20Z', {
        kind: 'backgroundTaskEnded',
        toolCallId: 'agent',
        failed: false
      }),
      tool('g', 'Grep')
    ])

    expect(shape(foldTurns(rows, true))).toEqual([
      'u1',
      'answered:start:1',
      'a1',
      'ended',
      'work'
    ])
  })

  test('folds a stopped turn whole, with what the agent said last in view', () => {
    const rows = groupTranscript([
      user('u1', '2026-11-08T10:00:00Z'),
      tool('r', 'Read'),
      answer('said', '2026-11-08T10:00:05Z'),
      marker('stop', '2026-11-08T10:00:12Z', { kind: 'turnInterrupted' })
    ])

    const folded = foldTurns(rows, false)

    expect(shape(folded)).toEqual(['u1', 'stopped:start:1', 'said'])
    const fold = folded[1]
    expect(fold?.kind === 'fold' && fold.durationMs).toBe(12_000)
  })

  test('folds settled work after the answer, and leaves a failed turn open', () => {
    const failed = { ...tool('x', 'Bash') }
    failed.content = { ...failed.content, isError: true }
    const start = [
      user('u1', '2026-11-08T10:00:00Z'),
      tool('r', 'Read'),
      answer('a1', '2026-11-08T10:00:05Z')
    ]
    expect(
      shape(foldTurns(groupTranscript([...start, tool('w', 'Write')]), false))
    ).toEqual(['u1', 'answered:start:2', 'a1'])
    expect(
      shape(foldTurns(groupTranscript([...start, failed]), false))
    ).toEqual(['u1', 'answered:start:1', 'a1', 'work'])

    const errored = groupTranscript([
      user('u1', '2026-11-08T10:00:00Z'),
      tool('r', 'Read'),
      answer('a1', '2026-11-08T10:00:05Z'),
      marker('fail', '2026-11-08T10:00:06Z', {
        kind: 'turnFailed',
        reason: { code: 'providerError', detail: 'x' }
      })
    ])
    expect(shape(foldTurns(errored, false))).toEqual([
      'u1',
      'work',
      'a1',
      'fail'
    ])
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

describe('turn changes', () => {
  const user = (id: string, steer = false): ThreadItem => ({
    id,
    threadId: 't',
    createdAt: '',
    content: {
      kind: 'userMessage',
      text: id,
      attachments: [],
      steer,
      taskResult: null
    }
  })
  const changes = (turnId: string, paths: string[] | null): ThreadItem => ({
    id: `${turnId}:changes`,
    threadId: 't',
    createdAt: '',
    content: {
      kind: 'turnChanges',
      files:
        paths?.map((path) => ({
          path,
          oldPath: null,
          status: 'modified',
          additions: 1,
          deletions: 0
        })) ?? null
    }
  })
  const changed = (rows: TranscriptRow[]) =>
    rows.flatMap((row) =>
      row.kind === 'item' && row.changes
        ? [[row.item.id, row.changes.turnId, row.changes.files.length]]
        : []
    )

  test('go on the turn’s last answer, past a steer', () => {
    const rows = groupTranscript([
      user('m1'),
      text('a1'),
      user('s1', true),
      tool('c1', 'Edit'),
      text('a2'),
      changes('m1', ['a.ts', 'b.ts'])
    ])

    expect(changed(rows)).toEqual([['a2', 'm1', 2]])
    expect(
      rows.some((row) => row.kind === 'item' && row.item.id === 'm1:changes')
    ).toBe(false)
  })

  test('are left out while read, when empty, or when the turn never answered', () => {
    const rows = groupTranscript([
      user('m1'),
      text('a1'),
      changes('m1', null),
      user('m2'),
      text('a2'),
      changes('m2', []),
      user('m3'),
      tool('c1', 'Edit'),
      changes('m3', ['a.ts'])
    ])

    expect(changed(rows)).toEqual([])
  })
})

describe('readingChanges', () => {
  const changes = (files: null | []): ThreadItem => ({
    id: 'u1:changes',
    threadId: 't',
    createdAt: '',
    content: { kind: 'turnChanges', files }
  })
  const asked: ThreadItem = {
    id: 'u2',
    threadId: 't',
    createdAt: '',
    content: {
      kind: 'userMessage',
      text: 'again',
      attachments: [],
      steer: false,
      taskResult: null,
      pullRequestUpdate: null,
      rewind: null
    }
  }

  test("tells whether the latest turn's changes are still read", () => {
    expect(readingChanges([text('a'), changes(null)])).toBe(true)
    expect(readingChanges([text('a'), changes([])])).toBe(false)
    expect(readingChanges([text('a')])).toBeNull()
  })

  test("an earlier turn's changes don't count", () => {
    expect(readingChanges([changes(null), asked, text('b')])).toBeNull()
  })
})

describe('statusAnchor', () => {
  const asked = (id: string, steer = false): ThreadItem => ({
    id,
    threadId: 't',
    createdAt: '',
    content: {
      kind: 'userMessage',
      text: id,
      attachments: [],
      steer,
      taskResult: null,
      pullRequestUpdate: null,
      rewind: null
    }
  })
  const item = (value: ThreadItem): TranscriptRow => ({
    kind: 'item',
    item: value
  })
  const work = (id: string): TranscriptRow => ({
    kind: 'work',
    id,
    entries: [tool(id, 'Read')]
  })

  test('follows what started the latest run', () => {
    const user = item(asked('u1'))
    const steer = item(asked('u2', true))
    expect(statusAnchor([user, work('w1'), item(text('a'))])).toBe(user)
    expect(statusAnchor([user, work('w1'), steer, work('w2')])).toBe(steer)
  })

  test('follows the fold of a run that ended before it, or goes first', () => {
    const fold: TranscriptRow = {
      kind: 'fold',
      id: 'fold:u1',
      rows: [],
      ending: 'answered',
      after: 'start',
      durationMs: null
    } as TranscriptRow
    expect(statusAnchor([item(asked('u1')), fold, work('w2')])).toBe(fold)
    expect(statusAnchor([work('w1')])).toBeNull()
  })
})

describe('streamingShape', () => {
  test('names what streams but not its text, which grows on its own', () => {
    const shape = streamingShape({
      a: { kind: 'text', text: 'Hel' },
      b: { kind: 'reasoning', text: 'Thinking' },
      c: { kind: 'toolOutput', text: 'log' }
    })
    expect(
      streamingShape({
        a: { kind: 'text', text: 'Hello there' },
        b: { kind: 'reasoning', text: 'Thinking more' },
        c: { kind: 'toolOutput', text: 'log' }
      })
    ).toBe(shape)
    expect(fromStreamingShape(shape)).toEqual({
      a: { kind: 'text', text: '' },
      b: { kind: 'reasoning', text: '' }
    })
    expect(fromStreamingShape('')).toEqual({})
  })
})
