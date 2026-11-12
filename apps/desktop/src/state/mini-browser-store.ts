import { create } from 'zustand'
import { withoutKeys } from '@/lib/records'
import type { Interaction, Point } from '@/lib/mini-browser-layout'

/** Where the user left a thread's floating browser; `null` until they move or resize it. */
export type MiniBrowser = {
  position: Point | null
  width: number | null
  interaction: Interaction
}

/** Room the chat column leaves the floating browser, as the chat lane's CSS variables take it. */
export type LaneRoom = { threadId: string; left: number; right: number }

type MiniBrowserState = {
  threads: Record<string, MiniBrowser>
  /** Set while the shown thread's floating browser moves the chat aside. */
  room: LaneRoom | null
  open: (threadId: string) => void
  close: (threadId: string) => void
  move: (threadId: string, position: Point) => void
  resize: (threadId: string, width: number, position: Point) => void
  setRoom: (room: LaneRoom | null) => void
  forget: (threadIds: string[]) => void
}

const PLACED_NOWHERE: MiniBrowser = {
  position: null,
  width: null,
  interaction: 'drag'
}

const sameRoom = (a: LaneRoom | null, b: LaneRoom | null) =>
  a === b ||
  (a !== null &&
    b !== null &&
    a.threadId === b.threadId &&
    a.left === b.left &&
    a.right === b.right)

export const useMiniBrowserStore = create<MiniBrowserState>()((set) => {
  const change = (
    threadId: string,
    next: (current: MiniBrowser) => MiniBrowser
  ) =>
    set((state) => {
      const current = state.threads[threadId]
      return current
        ? { threads: { ...state.threads, [threadId]: next(current) } }
        : state
    })
  return {
    threads: {},
    room: null,
    open: (threadId) =>
      set((state) =>
        state.threads[threadId]
          ? state
          : { threads: { ...state.threads, [threadId]: PLACED_NOWHERE } }
      ),
    close: (threadId) =>
      set((state) => {
        const threads = withoutKeys(state.threads, [threadId])
        return threads === state.threads ? state : { threads }
      }),
    move: (threadId, position) =>
      change(threadId, (current) => ({
        ...current,
        position,
        interaction: 'drag'
      })),
    resize: (threadId, width, position) =>
      change(threadId, () => ({ width, position, interaction: 'resize' })),
    setRoom: (room) =>
      set((state) => (sameRoom(state.room, room) ? state : { room })),
    forget: (threadIds) =>
      set((state) => {
        const threads = withoutKeys(state.threads, threadIds)
        return threads === state.threads ? state : { threads }
      })
  }
})
