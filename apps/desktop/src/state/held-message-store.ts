import { create } from 'zustand'
import type { HeldMessage } from '@/bindings/HeldMessage'
import type { ThreadHeldMessage } from '@/bindings/ThreadHeldMessage'

type HeldMessageState = {
  /** The message each thread holds until its conversation is compacted, by thread id. */
  byThread: Record<string, HeldMessage>
  seed: (held: HeldMessage[]) => void
  receive: (update: ThreadHeldMessage) => void
}

export const useHeldMessageStore = create<HeldMessageState>()((set) => ({
  byThread: {},
  seed: (held) =>
    set({
      byThread: Object.fromEntries(
        held.map((message) => [message.threadId, message])
      )
    }),
  receive: ({ threadId, message }) =>
    set((state) => {
      const { [threadId]: _, ...rest } = state.byThread
      return { byThread: message ? { ...rest, [threadId]: message } : rest }
    })
}))
