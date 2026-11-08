import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import { invoke } from '@tauri-apps/api/core'
import { useTerminalStore } from './terminal-store'
import {
  activateSurface,
  closeSurface,
  NO_SURFACES,
  openSurface,
  type Surface,
  type SurfaceKind,
  type ThreadSurfaces,
  updateSurface
} from './panel-surfaces'

export const PANEL_WIDTH = { min: 320, max: 760, default: 440 } as const

type PanelState = {
  open: boolean
  /** The thread details card, floating over the chat or as a popover when it doesn't fit. */
  detailsOpen: boolean
  /** The panel takes the chat's place, for a wide diff or a terminal. */
  maximized: boolean
  width: number
  /** Open surfaces by thread id. */
  threads: Record<string, ThreadSurfaces>
  toggle: () => void
  setDetailsOpen: (detailsOpen: boolean) => void
  close: () => void
  toggleMaximized: () => void
  setWidth: (width: number) => void
  /** Opens the panel on a surface of that kind, reusing a single-instance one. */
  openSurface: (threadId: string, kind: SurfaceKind) => void
  activateSurface: (threadId: string, surfaceId: string) => void
  /** Closing the last surface closes the panel too. */
  closeSurface: (threadId: string, surfaceId: string) => void
  updateSurface: <S extends Surface>(
    threadId: string,
    surfaceId: string,
    change: (surface: S) => S
  ) => void
}

const makeId = () => crypto.randomUUID()

export const usePanelStore = create<PanelState>()(
  persist(
    (set, get) => {
      const changeThread = (
        threadId: string,
        change: (panel: ThreadSurfaces) => ThreadSurfaces
      ) =>
        set((state) => ({
          threads: {
            ...state.threads,
            [threadId]: change(state.threads[threadId] ?? NO_SURFACES)
          }
        }))
      return {
        open: false,
        detailsOpen: false,
        maximized: false,
        width: PANEL_WIDTH.default,
        threads: {},
        toggle: () => set((state) => ({ open: !state.open })),
        setDetailsOpen: (detailsOpen) => set({ detailsOpen }),
        close: () => set({ open: false, maximized: false }),
        toggleMaximized: () =>
          set((state) => ({ maximized: !state.maximized })),
        setWidth: (width) =>
          set({
            width: Math.round(
              Math.min(PANEL_WIDTH.max, Math.max(PANEL_WIDTH.min, width))
            )
          }),
        openSurface: (threadId, kind) => {
          changeThread(threadId, (panel) => openSurface(panel, kind, makeId))
          set({ open: true })
          const activeId = get().threads[threadId]?.activeId
          if (kind === 'terminal' && activeId) {
            useTerminalStore.getState().requestFocus(activeId)
          }
        },
        activateSurface: (threadId, surfaceId) =>
          changeThread(threadId, (panel) => activateSurface(panel, surfaceId)),
        closeSurface: (threadId, surfaceId) => {
          const closing = get().threads[threadId]?.surfaces.find(
            (surface) => surface.id === surfaceId
          )
          if (closing?.kind === 'terminal') {
            void invoke('close_terminal', { terminalId: surfaceId })
          }
          changeThread(threadId, (panel) => closeSurface(panel, surfaceId))
          set((state) =>
            state.threads[threadId]?.surfaces.length === 0
              ? { open: false, maximized: false }
              : {}
          )
        },
        updateSurface: (threadId, surfaceId, change) =>
          changeThread(threadId, (panel) =>
            updateSurface(panel, surfaceId, change)
          )
      }
    },
    {
      name: 'hoshi-panel',
      partialize: ({ open, detailsOpen, maximized, width, threads }) => ({
        open,
        detailsOpen,
        maximized,
        width,
        threads
      })
    }
  )
)

/** The surfaces of a thread; stable while nothing changes, so selecting it is cheap. */
export function useThreadSurfaces(threadId: string): ThreadSurfaces {
  return usePanelStore((state) => state.threads[threadId] ?? NO_SURFACES)
}
