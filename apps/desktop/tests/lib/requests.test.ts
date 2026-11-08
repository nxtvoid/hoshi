import { describe, expect, test } from 'bun:test'
import type { ThreadItem } from '../../src/bindings/ThreadItem'
import type { RequestStatus } from '../../src/bindings/RequestStatus'
import {
  answerQuestion,
  approvalsByCall,
  answerText,
  pendingRequest,
  previousQuestion,
  START,
  toggleOption,
  type QuestionsContent
} from '../../src/lib/requests'

function approval(id: string, status: RequestStatus): ThreadItem {
  return {
    id,
    threadId: 't',
    createdAt: '',
    content: {
      kind: 'approval',
      toolCallId: null,
      toolName: 'Bash',
      input: '{}',
      reason: null,
      canAcceptForSession: true,
      status,
      decision: null,
      feedback: null
    }
  }
}

const questions: QuestionsContent = {
  kind: 'questions',
  status: 'pending',
  answers: null,
  questions: [
    {
      header: 'Color',
      question: 'Which color?',
      multiSelect: false,
      options: [
        { label: 'Red', description: '', preview: null },
        { label: 'Blue', description: '', preview: null }
      ]
    },
    {
      header: 'Sizes',
      question: 'Which sizes?',
      multiSelect: true,
      options: [
        { label: 'S', description: '', preview: null },
        { label: 'M', description: '', preview: null }
      ]
    }
  ]
}

describe('approvalsByCall', () => {
  test('pairs an approval with the call it held back, when the call is shown', () => {
    const call: ThreadItem = {
      id: 'toolu_1',
      threadId: 't',
      createdAt: '',
      content: {
        kind: 'toolCall',
        name: 'Bash',
        input: '{}',
        output: null,
        isError: false,
        detail: null,
        outputOmitted: false,
        interrupted: false
      }
    }
    const held = approval('a', 'resolved')
    const onCall = {
      ...held,
      content: { ...held.content, toolCallId: 'toolu_1' }
    } as ThreadItem
    const elsewhere = {
      ...held,
      id: 'b',
      content: { ...held.content, toolCallId: 'toolu_9' }
    } as ThreadItem

    const approvals = approvalsByCall([call, onCall, elsewhere])

    expect([...approvals.keys()]).toEqual(['toolu_1'])
  })
})

describe('pendingRequest', () => {
  test('is the oldest request still waiting', () => {
    const items = [
      approval('a', 'resolved'),
      approval('b', 'pending'),
      approval('c', 'pending')
    ]
    expect(pendingRequest(items)?.id).toBe('b')
    expect(pendingRequest([approval('a', 'cancelled')])).toBeNull()
  })
})

describe('questions', () => {
  test('a single choice keeps the latest pick, a multiple one toggles', () => {
    let progress = toggleOption(START, 'Red', false)
    progress = toggleOption(progress, 'Blue', false)
    expect(progress.selected).toEqual(['Blue'])

    progress = toggleOption(progress, 'S', true)
    progress = toggleOption(progress, 'M', true)
    progress = toggleOption(progress, 'S', true)
    expect(progress.selected).toEqual(['Blue', 'M'])
  })

  test('typed text wins over the picked options', () => {
    expect(answerText(['S', 'M'], '')).toBe('S, M')
    expect(answerText(['S'], '  extra large ')).toBe('extra large')
  })

  test('answers every question in turn, then returns them all', () => {
    const first = answerQuestion(START, 2, 'Red')
    expect(first.answers).toBeNull()
    expect(first.progress).toEqual({ index: 1, answers: ['Red'], selected: [] })

    const last = answerQuestion(first.progress, 2, 'S, M')
    expect(last.answers).toEqual(['Red', 'S, M'])
  })

  test('an empty answer does not move on', () => {
    expect(answerQuestion(START, 2, '  ').progress).toBe(START)
  })

  test('going back restores what was picked there', () => {
    const { progress } = answerQuestion(START, 2, 'Blue')
    expect(previousQuestion(progress, questions)).toEqual({
      index: 0,
      answers: [],
      selected: ['Blue']
    })
    expect(previousQuestion(START, questions)).toBe(START)
  })
})
