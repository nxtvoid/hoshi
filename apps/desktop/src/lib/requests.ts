import type { ThreadItem } from '@/bindings/ThreadItem'
import type { ItemContent } from '@/bindings/ItemContent'

export type RequestContent = Extract<
  ItemContent,
  { kind: 'approval' | 'questions' | 'proposedPlan' | 'secretRequest' }
>
export type RequestItem = ThreadItem & { content: RequestContent }
export type QuestionsContent = Extract<ItemContent, { kind: 'questions' }>
export type ApprovalContent = Extract<ItemContent, { kind: 'approval' }>
export type SecretRequestContent = Extract<
  ItemContent,
  { kind: 'secretRequest' }
>

/**
 * Approvals by the tool call they held back, which shows them on its own row: in the
 * thread's transcript, or a subagent's.
 */
export function approvalsByCall(
  items: ThreadItem[]
): Map<string, ApprovalContent> {
  const approvals = new Map<string, ApprovalContent>()
  for (const { content } of items) {
    if (content.kind === 'approval' && content.toolCallId !== null) {
      approvals.set(content.toolCallId, content)
    }
  }
  return approvals
}

/** Tools whose call is shown by what it makes, a request or the task list, not as a call. */
export const REQUEST_TOOLS = new Set([
  'AskUserQuestion',
  'ExitPlanMode',
  'TodoWrite',
  'TaskCreate',
  'TaskUpdate',
  'TaskList',
  'TaskGet',
  'mcp__hoshi__request_secret'
])

export function isRequest(item: ThreadItem): item is RequestItem {
  const { kind } = item.content
  return (
    kind === 'approval' ||
    kind === 'questions' ||
    kind === 'proposedPlan' ||
    kind === 'secretRequest'
  )
}

/** The oldest request still waiting for the user, which is the one to answer first. */
export function pendingRequest(items: ThreadItem[]): RequestItem | null {
  for (const item of items) {
    if (isRequest(item) && item.content.status === 'pending') return item
  }
  return null
}

/** Where the user is in a set of questions, and what they answered so far. */
export type QuestionProgress = {
  index: number
  answers: string[]
  /** The options picked on the current question. */
  selected: string[]
}

export const START: QuestionProgress = { index: 0, answers: [], selected: [] }

/** Picks or unpicks an option; a single-choice question keeps only the latest. */
export function toggleOption(
  progress: QuestionProgress,
  label: string,
  multiSelect: boolean
): QuestionProgress {
  if (!multiSelect) return { ...progress, selected: [label] }
  const selected = progress.selected.includes(label)
    ? progress.selected.filter((picked) => picked !== label)
    : [...progress.selected, label]
  return { ...progress, selected }
}

/** The user's own text wins over the picked options, which join with ", ". */
export function answerText(selected: string[], custom: string) {
  const text = custom.trim()
  return text === '' ? selected.join(', ') : text
}

/**
 * Records the answer to the current question and moves to the next. Returns every answer
 * once the last question is answered; an empty answer changes nothing.
 */
export function answerQuestion(
  progress: QuestionProgress,
  total: number,
  answer: string
): { progress: QuestionProgress; answers: string[] | null } {
  if (answer.trim() === '') return { progress, answers: null }
  const answers = [...progress.answers.slice(0, progress.index), answer]
  if (answers.length >= total) return { progress, answers }
  return {
    progress: { index: progress.index + 1, answers, selected: [] },
    answers: null
  }
}

/** Back to the previous question, with what was picked there. */
export function previousQuestion(
  progress: QuestionProgress,
  content: QuestionsContent
): QuestionProgress {
  if (progress.index === 0) return progress
  const index = progress.index - 1
  const answer = progress.answers[index] ?? ''
  const labels = content.questions[index]?.options.map((option) => option.label)
  const picked = answer.split(', ').filter((part) => labels?.includes(part))
  return { index, answers: progress.answers.slice(0, index), selected: picked }
}
