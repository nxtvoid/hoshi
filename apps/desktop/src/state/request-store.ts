import { create } from 'zustand'
import type { QuestionProgress } from '@/lib/requests'

type RequestState = {
  /** How far the user got through each request's questions, by request id. */
  progress: Record<string, QuestionProgress>
  setProgress: (requestId: string, progress: QuestionProgress) => void
}

export const useRequestStore = create<RequestState>()((set) => ({
  progress: {},
  setProgress: (requestId, progress) =>
    set((state) => ({ progress: { ...state.progress, [requestId]: progress } }))
}))
