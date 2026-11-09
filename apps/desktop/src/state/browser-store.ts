import { create } from 'zustand'
import { useAppStore } from './app-store'
import { usePanelStore } from './panel-store'
import type { BrowserPage } from '@/bindings/BrowserPage'
import type { BrowserFrame } from '@/bindings/BrowserFrame'

export type ShownPage = BrowserPage & {
  /** When the agent last acted on it, from `Date.now()`. */
  agentAt: number | null
}

type BrowserState = {
  /** The latest frame of each thread's page, while the panel shows it. */
  frames: Record<string, BrowserFrame>
  pages: Record<string, ShownPage>
  receiveFrame: (frame: BrowserFrame) => void
  receivePage: (page: BrowserPage) => void
}

const without = <T>(record: Record<string, T>, key: string) => {
  const { [key]: _, ...rest } = record
  return rest
}

export const useBrowserStore = create<BrowserState>()((set, get) => ({
  frames: {},
  pages: {},
  receiveFrame: (frame) =>
    set((state) => ({ frames: { ...state.frames, [frame.threadId]: frame } })),
  receivePage: (page) => {
    if (!page.open) {
      set((state) => ({
        frames: without(state.frames, page.threadId),
        pages: without(state.pages, page.threadId)
      }))
      return
    }
    const before = get().pages[page.threadId]
    const agentAt = page.byAgent ? Date.now() : (before?.agentAt ?? null)
    const fitted = page.fitted ?? before?.fitted ?? null
    set((state) => ({
      pages: { ...state.pages, [page.threadId]: { ...page, agentAt, fitted } }
    }))
    if (
      page.opened &&
      page.threadId === useAppStore.getState().selectedThreadId
    ) {
      usePanelStore.getState().openSurface(page.threadId, 'browser')
    }
  }
}))
