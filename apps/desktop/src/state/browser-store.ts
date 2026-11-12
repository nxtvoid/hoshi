import { create } from 'zustand'
import { usePanelStore } from './panel-store'
import { useSettingsStore } from './settings-store'
import { useMiniBrowserStore } from './mini-browser-store'
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
      useMiniBrowserStore.getState().close(page.threadId)
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
    if (!page.opened || panelShowsBrowser(page.threadId)) return
    if (floatsAgentPages()) useMiniBrowserStore.getState().open(page.threadId)
    else usePanelStore.getState().placeSurface(page.threadId, 'browser')
  }
}))

function floatsAgentPages() {
  return useSettingsStore.getState().settings?.browser.floatAgentPages ?? true
}

function panelShowsBrowser(threadId: string) {
  const panel = usePanelStore.getState()
  const surfaces = panel.threads[threadId]
  const active = surfaces?.surfaces.find(
    (surface) => surface.id === surfaces.activeId
  )
  return Boolean(panel.openThreads[threadId]) && active?.kind === 'browser'
}
