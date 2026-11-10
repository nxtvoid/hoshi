import { expect, test } from 'bun:test'
import { join } from 'node:path'
import { existsSync, readdirSync, readFileSync } from 'node:fs'
import type { Thread } from '../../src/bindings/Thread'
import { foldTaskRuns } from '../../src/lib/task-runs'
import type { ThreadItem } from '../../src/bindings/ThreadItem'
import { useRequestStore } from '../../src/state/request-store'
import type { EventEnvelope } from '../../src/bindings/EventEnvelope'
import { applyEvent, applyItemEvent } from '../../src/state/orchestration'
import { useBackgroundTaskStore } from '../../src/state/background-task-store'
import type { ThreadBackgroundTasks } from '../../src/bindings/ThreadBackgroundTasks'
import {
  type AgentState,
  busyThreads,
  inFlight,
  runningTasks,
  taskState
} from '../../src/lib/agents'
import {
  foldTurns,
  groupTranscript,
  staysInView,
  type TranscriptRow
} from '../../src/lib/tool-calls'

/**
 * What real runs sent the UI, recorded by `delegation_scenarios` in hoshi-engine. Each is
 * replayed one event at a time through the UI's own logic, which may render between any
 * two of them, and has to make sense at every step, whatever the agents chose to do.
 */
type Step = {
  at: number
  event?: EventEnvelope
  backgroundTasks?: ThreadBackgroundTasks
}
type Recording = {
  name: string
  prompt: string
  timedOut: boolean
  steps: Step[]
}

type Stretch = { folded: boolean; notes: number; tail: string }
type Seen = {
  at: number
  tasks: Record<string, { flying: boolean; state: AgentState }>
  /** By the message that started each: how the stretches of threads with tasks show. */
  stretches: Record<string, Stretch>
  waiting: Record<string, string[]>
}
type Answer = {
  at: number
  task: string
  text: string
  taskFlying: boolean
  othersFlying: number
}

const DIR = join(import.meta.dir, '../fixtures/task-runs')
const recordings: Recording[] = existsSync(DIR)
  ? readdirSync(DIR)
      .filter((file) => file.endsWith('.json'))
      .map((file) => JSON.parse(readFileSync(join(DIR, file), 'utf8')))
  : []

function startsStretch(row: TranscriptRow) {
  if (row.kind !== 'item') return false
  const { content } = row.item
  return content.kind === 'userMessage' && !content.steer && !content.taskResult
}

function stretchesOf(rows: TranscriptRow[]): Record<string, Stretch> {
  const stretches: Record<string, Stretch> = {}
  let current: Stretch | null = null
  for (const row of rows) {
    if (startsStretch(row) && row.kind === 'item') {
      current = { folded: false, notes: 0, tail: '' }
      stretches[row.item.id] = current
    } else if (!current) {
    } else if (row.kind === 'fold') {
      current.folded ||= row.id.startsWith('tasks:')
    } else if (
      row.kind === 'item' &&
      row.item.content.kind === 'assistantMessage'
    ) {
      if (row.note) current.notes += 1
      else current.tail = row.item.content.text
    }
  }
  return stretches
}

function replay(recording: Recording) {
  useRequestStore.getState().seed([], [])
  useBackgroundTaskStore.getState().seed([])
  let state: { projects: never[]; threads: Thread[] } = {
    projects: [],
    threads: []
  }
  const items: Record<string, ThreadItem[]> = {}
  const seen: Seen[] = []
  const answers: Answer[] = []
  const shown: unknown[] = []

  for (const step of recording.steps) {
    const event = step.event?.event
    if (event) {
      state = applyEvent(state, event) as typeof state
      useRequestStore.getState().applyEvent(event)
      for (const thread of state.threads) {
        items[thread.id] = applyItemEvent(
          thread.id,
          items[thread.id] ?? [],
          event
        )
      }
    }
    if (step.backgroundTasks) {
      useBackgroundTaskStore.getState().receive(step.backgroundTasks)
    }
    const { threads } = state
    const { pendingTasks } = useRequestStore.getState()
    const busy = busyThreads(
      threads,
      pendingTasks,
      useBackgroundTaskStore.getState().byThread
    )
    const flying = (thread: Thread) => inFlight(thread, pendingTasks, busy)
    const tasks = threads.filter((thread) => thread.parentThreadId !== null)
    const now: Seen = { at: step.at, tasks: {}, stretches: {}, waiting: {} }
    for (const task of tasks) {
      now.tasks[task.id] = {
        flying: flying(task),
        state: taskState(task, pendingTasks, busy)
      }
    }
    const parents = new Set(tasks.map((task) => task.parentThreadId as string))
    for (const parent of threads.filter((thread) => parents.has(thread.id))) {
      const working =
        parent.status === 'working' || parent.status === 'waitingForInput'
      const rows = foldTaskRuns(
        foldTurns(
          groupTranscript(items[parent.id] ?? []),
          working,
          staysInView
        ),
        (taskId) =>
          tasks.some(
            (task) =>
              task.id === taskId &&
              task.parentThreadId === parent.id &&
              flying(task)
          )
      )
      expect(
        rows.filter(
          (row) =>
            row.kind === 'item' &&
            row.item.content.kind === 'userMessage' &&
            row.item.content.taskResult
        )
      ).toEqual([])
      Object.assign(now.stretches, stretchesOf(rows))
      now.waiting[parent.id] = runningTasks(
        parent.id,
        threads,
        pendingTasks,
        busy
      ).map((task) => task.title)
    }
    seen.push(now)

    if (
      event?.type === 'itemAdded' &&
      event.item.content.kind === 'userMessage' &&
      event.item.content.taskResult
    ) {
      const { taskResult, text } = event.item.content
      const parent = event.item.threadId
      answers.push({
        at: step.at,
        task: taskResult,
        text,
        taskFlying: now.tasks[taskResult]?.flying ?? false,
        othersFlying: tasks.filter(
          (task) =>
            task.id !== taskResult &&
            task.parentThreadId === parent &&
            flying(task)
        ).length
      })
    }

    if (process.env.TASK_RUN_TIMELINE) {
      const title = (id: string) =>
        threads.find((thread) => thread.id === id)?.title.slice(0, 28) ?? id
      const line = JSON.stringify({
        tasks: Object.fromEntries(
          Object.entries(now.tasks).map(([id, task]) => [title(id), task.state])
        ),
        stretches: Object.values(now.stretches).map(
          (stretch) =>
            `${stretch.folded ? 'folded' : 'open'} notes=${stretch.notes} tail=${JSON.stringify(stretch.tail.slice(0, 50))}`
        ),
        waiting: now.waiting
      })
      if (line !== shown.at(-1)) {
        shown.push(line)
        console.log(
          `${(step.at / 1000).toFixed(1).padStart(6)}s ${event?.type ?? 'background'}\n        ${line}`
        )
      }
    }
  }
  return { seen, answers, threads: state.threads, items }
}

test.each(recordings)('the UI makes sense all along: $name', (recording) => {
  expect(recording.timedOut).toBe(false)
  const { seen, answers, threads, items } = replay(recording)
  const title = (id: string) =>
    threads.find((thread) => thread.id === id)?.title ?? id

  for (const task of threads.filter((thread) => thread.parentThreadId)) {
    const flights = seen.map((step) => step.tasks[task.id]?.flying ?? false)
    const landed = flights.indexOf(false, flights.indexOf(true))
    expect({
      task: title(task.id),
      fliesAgainAt:
        landed === -1 || flights.indexOf(true, landed) === -1
          ? null
          : seen[flights.indexOf(true, landed)]?.at
    }).toEqual({ task: title(task.id), fliesAgainAt: null })

    const states = seen.map((step) => step.tasks[task.id]?.state)
    const done = states.indexOf('done')
    const again = states.findIndex(
      (state, index) =>
        done !== -1 &&
        index > done &&
        (state === 'running' || state === 'waiting')
    )
    expect({
      task: title(task.id),
      runsAfterDoneAt: again === -1 ? null : seen[again]?.at
    }).toEqual({ task: title(task.id), runsAfterDoneAt: null })
  }

  const folded = new Set<string>()
  for (const step of seen) {
    for (const [id, stretch] of Object.entries(step.stretches)) {
      if (stretch.folded) folded.add(id)
      expect({
        stretch: id,
        unfoldsAt: step.at,
        wrong: folded.has(id)
      }).toEqual({ stretch: id, unfoldsAt: step.at, wrong: stretch.folded })
    }
  }

  for (const answer of answers) {
    const says = {
      taskAnswersAgain: answer.text.includes("It isn't finished"),
      othersStillRun: /\d+ other tasks? still runs?/.test(answer.text),
      allAnswered: answer.text.includes(
        'None of the tasks you delegated is still running'
      )
    }
    expect({ task: title(answer.task), at: answer.at, ...says }).toEqual({
      task: title(answer.task),
      at: answer.at,
      taskAnswersAgain: answer.taskFlying,
      othersStillRun: answer.othersFlying > 0,
      allAnswered: !answer.taskFlying && answer.othersFlying === 0
    })
  }

  const last = seen.at(-1)
  expect(threads.filter((thread) => thread.status !== 'idle')).toEqual([])
  expect(
    Object.values(last?.tasks ?? {}).filter((task) => task.flying)
  ).toEqual([])
  expect(Object.values(last?.waiting ?? {}).flat()).toEqual([])
  for (const parent of threads) {
    const results = (items[parent.id] ?? []).filter(
      (item) =>
        item.content.kind === 'userMessage' && item.content.taskResult !== null
    )
    if (results.length === 0) continue
    const rows = groupTranscript(items[parent.id] ?? [])
    const lastResult = rows.findLastIndex(
      (row) => row.kind === 'item' && row.item.id === results.at(-1)?.id
    )
    const start = rows
      .slice(0, lastResult)
      .findLast((row) => startsStretch(row))
    const stretch =
      start?.kind === 'item' ? last?.stretches[start.item.id] : undefined
    expect({
      thread: parent.title,
      folded: stretch?.folded,
      answers: (stretch?.tail ?? '') !== ''
    }).toEqual({ thread: parent.title, folded: true, answers: true })
  }
})
