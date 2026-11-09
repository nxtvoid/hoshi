import { create } from 'zustand'
import type { MessageAttachment } from '@/bindings/MessageAttachment'

/** A message to put back in a thread's composer, as after going back to before it. */
export type ComposerFill = {
  threadId: string
  text: string
  attachments: MessageAttachment[]
}

type ComposerFillState = {
  fill: ComposerFill | null
  request: (fill: ComposerFill) => void
  /** The composer took it in. */
  done: () => void
}

export const useComposerFillStore = create<ComposerFillState>()((set) => ({
  fill: null,
  request: (fill) => set({ fill }),
  done: () => set({ fill: null })
}))
