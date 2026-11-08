import { useAppStore } from '@/state/app-store'
import { useTranscriptStore } from '@/state/transcript-store'
import { useRequestStore } from '@/state/request-store'
import type { ApprovalDecision } from '@/bindings/ApprovalDecision'
import {
  answerQuestion,
  answerText,
  pendingRequest,
  previousQuestion,
  START,
  toggleOption,
  type QuestionProgress,
  type RequestItem
} from '@/lib/requests'

/** The request the selected thread's agent waits on, oldest first. */
export function usePendingRequest() {
  return useTranscriptStore((state) => pendingRequest(state.items))
}

/** Answers `request`; shared by its panel and the composer, which takes typed answers. */
export function useRequestActions(request: RequestItem | null) {
  const respond = useAppStore((state) => state.respondToRequest)
  const progress = useRequestStore((state) =>
    request ? state.progress[request.id] : undefined
  )
  const setProgress = useRequestStore((state) => state.setProgress)
  const current = progress ?? START

  const decide = (
    decision: ApprovalDecision,
    feedback: string | null = null
  ) =>
    request
      ? respond(request.id, { type: 'decision', decision, feedback })
      : Promise.resolve(false)

  /** Answers the current question and moves on; `false` when there was nothing to answer. */
  const advanceFrom = (from: QuestionProgress, custom: string) => {
    if (request?.content.kind !== 'questions') return false
    const answer = answerText(from.selected, custom)
    const result = answerQuestion(
      from,
      request.content.questions.length,
      answer
    )
    if (result.answers) {
      void respond(request.id, { type: 'answers', answers: result.answers })
      return true
    }
    setProgress(request.id, result.progress)
    return result.progress !== from
  }

  const pick = (label: string, multiSelect: boolean) => {
    if (!request) return
    const next = toggleOption(current, label, multiSelect)
    if (multiSelect) setProgress(request.id, next)
    else advanceFrom(next, '')
  }

  const back = () => {
    if (request?.content.kind !== 'questions') return
    setProgress(request.id, previousQuestion(current, request.content))
  }

  return {
    progress: current,
    decide,
    pick,
    back,
    advance: (custom = '') => advanceFrom(current, custom)
  }
}
