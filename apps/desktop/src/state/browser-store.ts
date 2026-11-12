import { create } from 'zustand'
import { usePanelStore } from './panel-store'
import { useSettingsStore } from './settings-store'
import { useMiniBrowserStore } from './mini-browser-store'
import type { BrowserPage } from '@/bindings/BrowserPage'
import type { RawBrowserFrame } from '@/lib/browser-frame'

export type ShownPage = BrowserPage & {
  /** When the agent last acted on it, from `Date.now()`. */
  agentAt: number | null
}

/** A frame on screen: its JPEG behind an object URL, and the page's size in CSS pixels. */
export type ShownFrame = { url: string; width: number; height: number }

type BrowserState = {
  /** The latest frame of each thread's page, while the panel shows it. */
  frames: Record<string, ShownFrame>
  pages: Record<string, ShownPage>
  receiveFrame: (frame: RawBrowserFrame) => void
  receivePage: (page: BrowserPage) => void
}

const without = <T>(record: Record<string, T>, key: string) => {
  const { [key]: _, ...rest } = record
  return rest
}

/** How long a replaced frame's URL stays valid, so an image still showing it can finish. */
const REVOKE_AFTER_MS = 1000

function release(frame: ShownFrame | undefined) {
  if (frame) setTimeout(() => URL.revokeObjectURL(frame.url), REVOKE_AFTER_MS)
}

export const useBrowserStore = create<BrowserState>()((set, get) => ({
  frames: {},
  pages: {},
  receiveFrame: ({ threadId, width, height, jpeg }) => {
    const before = get().frames[threadId]
    const url = URL.createObjectURL(new Blob([jpeg], { type: 'image/jpeg' }))
    set((state) => ({
      frames: { ...state.frames, [threadId]: { url, width, height } }
    }))
    release(before)
  },
  receivePage: (page) => {
    if (!page.open) {
      useMiniBrowserStore.getState().close(page.threadId)
      release(get().frames[page.threadId])
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
