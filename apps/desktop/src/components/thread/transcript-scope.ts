import { createContext, useContext } from 'react'
import type { ThreadItem } from '@/bindings/ThreadItem'
import type { StreamingText } from '@/lib/tool-calls'
import { keptTranscript, useTranscriptStore } from '@/state/transcript-store'

/** What a transcript view shows of its thread. */
export type TranscriptView = {
  threadId: string | null
  items: ThreadItem[]
  hasOlder: boolean
  streaming: Record<string, StreamingText>
  toolOutput: Record<string, string>
  /** It is the transcript on screen, which loads older items and streams. */
  shown: boolean
}

/**
 * The thread a mounted transcript belongs to; `null` follows whichever is shown. A hidden
 * one reads its thread as kept in memory, so nothing in it renders again until it shows.
 */
export const TranscriptScope = createContext<string | null>(null)

const NO_ITEMS: ThreadItem[] = []
const NO_STREAMING: Record<string, StreamingText> = {}
const NO_OUTPUT: Record<string, string> = {}

/** Picks from the transcript of the thread in scope. */
export function useTranscriptView<T>(pick: (view: TranscriptView) => T): T {
  const scope = useContext(TranscriptScope)
  return useTranscriptStore((state) => {
    if (scope === null || state.shownId === scope) {
      const live = state.threadId === state.shownId
      return pick({
        threadId: state.shownId,
        items: state.items,
        hasOlder: state.hasOlder,
        streaming: live ? state.streaming : NO_STREAMING,
        toolOutput: live ? state.toolOutput : NO_OUTPUT,
        shown: true
      })
    }
    const kept = keptTranscript(scope)
    return pick({
      threadId: scope,
      items: kept?.items ?? NO_ITEMS,
      hasOlder: kept?.hasOlder ?? false,
      streaming: NO_STREAMING,
      toolOutput: NO_OUTPUT,
      shown: false
    })
  })
}
