import { useMemo } from 'react'
import { useAppStore } from '@/state/app-store'
import { selectQuietThreads } from '@/state/quiet-threads'
import type { Thread } from '@/bindings/Thread'
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

/**
 * The oldest request of a task the thread delegated, however deep, which the user can
 * answer from the thread, with the task's thread.
 */
export function useDelegatedRequest(threadId: string | null) {
  const threads = useAppStore(selectQuietThreads)
  const pending = useRequestStore((state) => state.pending)
  return useMemo(() => {
    if (!threadId) return null
    const byId = new Map(threads.map((thread) => [thread.id, thread]))
    const worksFor = (task: Thread) => {
      let parentId = task.parentThreadId
      for (let depth = 0; parentId && depth < threads.length; depth++) {
        if (parentId === threadId) return true
        parentId = byId.get(parentId)?.parentThreadId ?? null
      }
      return false
    }
    for (const request of pending) {
      const child = byId.get(request.threadId)
      if (child && worksFor(child)) return { request, child }
    }
    return null
  }, [threadId, threads, pending])
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
      ? respond(request.threadId, request.id, {
          type: 'decision',
          decision,
          feedback
        })
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
      void respond(request.threadId, request.id, {
        type: 'answers',
        answers: result.answers
      })
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
