import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import { withoutKeys } from '@/lib/records'
import { invoke } from '@tauri-apps/api/core'
import { useTerminalStore } from './terminal-store'
import {
  activateSurface,
  closeSurface,
  knownSurfaces,
  NO_SURFACES,
  openSurface,
  showTurnDiff,
  type Surface,
  type SurfaceKind,
  type ThreadSurfaces,
  updateSurface
} from './panel-surfaces'

export const PANEL_WIDTH = { min: 320, max: 760, default: 440 } as const

type PanelState = {
  /** Threads whose panel is open. */
  openThreads: Record<string, true>
  /** The thread details card, floating over the chat or as a popover when it doesn't fit. */
  detailsOpen: boolean
  /** The panel takes the chat's place, for a wide diff or a terminal. */
  maximized: boolean
  width: number
  /** Open surfaces by thread id. */
  threads: Record<string, ThreadSurfaces>
  toggle: (threadId: string) => void
  setDetailsOpen: (detailsOpen: boolean) => void
  close: (threadId: string) => void
  toggleMaximized: () => void
  setWidth: (width: number) => void
  /** Opens the panel on a surface of that kind, reusing a single-instance one. */
  openSurface: (threadId: string, kind: SurfaceKind) => void
  /** Makes it the panel's tab, as `openSurface` does, without opening the panel. */
  placeSurface: (threadId: string, kind: SurfaceKind) => void
  /** Opens the panel on what a turn changed, at `focusPath` when given. */
  openTurnDiff: (threadId: string, turnId: string, focusPath?: string) => void
  activateSurface: (threadId: string, surfaceId: string) => void
  /** Closing the last surface closes the panel too. */
  closeSurface: (threadId: string, surfaceId: string) => void
  updateSurface: <S extends Surface>(
    threadId: string,
    surfaceId: string,
    change: (surface: S) => S
  ) => void
  forget: (threadIds: string[]) => void
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
      const setOpen = (threadId: string, open: boolean) =>
        set((state) => {
          if (Boolean(state.openThreads[threadId]) === open) return state
          return {
            openThreads: open
              ? { ...state.openThreads, [threadId]: true }
              : withoutKeys(state.openThreads, [threadId]),
            ...(open ? {} : { maximized: false })
          }
        })
      return {
        openThreads: {},
        detailsOpen: false,
        maximized: false,
        width: PANEL_WIDTH.default,
        threads: {},
        toggle: (threadId) => setOpen(threadId, !get().openThreads[threadId]),
        setDetailsOpen: (detailsOpen) => set({ detailsOpen }),
        close: (threadId) => setOpen(threadId, false),
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
          setOpen(threadId, true)
          const activeId = get().threads[threadId]?.activeId
          if (kind === 'terminal' && activeId) {
            useTerminalStore.getState().requestFocus(activeId)
          }
        },
        placeSurface: (threadId, kind) =>
          changeThread(threadId, (panel) => openSurface(panel, kind, makeId)),
        openTurnDiff: (threadId, turnId, focusPath) => {
          changeThread(threadId, (panel) =>
            showTurnDiff(panel, turnId, focusPath, makeId)
          )
          setOpen(threadId, true)
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
          if (closing?.kind === 'browser') {
            void invoke('browser_close', { threadId })
          }
          changeThread(threadId, (panel) => closeSurface(panel, surfaceId))
          if (get().threads[threadId]?.surfaces.length === 0) {
            setOpen(threadId, false)
          }
        },
        updateSurface: (threadId, surfaceId, change) =>
          changeThread(threadId, (panel) =>
            updateSurface(panel, surfaceId, change)
          ),
        forget: (threadIds) =>
          set((state) => {
            const threads = withoutKeys(state.threads, threadIds)
            const openThreads = withoutKeys(state.openThreads, threadIds)
            return threads === state.threads &&
              openThreads === state.openThreads
              ? state
              : { threads, openThreads }
          })
      }
    },
    {
      name: 'hoshi-panel',
      merge: (saved, current) => {
        type Saved = Partial<PanelState> & { open?: boolean }
        const { open: _, ...restored } = (saved ?? {}) as Saved
        return {
          ...current,
          ...restored,
          openThreads: restored.openThreads ?? {},
          threads: knownSurfaces(restored.threads ?? {})
        }
      },
      partialize: ({
        openThreads,
        detailsOpen,
        maximized,
        width,
        threads
      }) => ({
        openThreads,
        detailsOpen,
        maximized,
        width,
        threads
      })
    }
  )
)

/** Whether the thread's panel is open. */
export function usePanelOpen(threadId: string | undefined): boolean {
  return usePanelStore((state) =>
    threadId ? Boolean(state.openThreads[threadId]) : false
  )
}

/** The surfaces of a thread; stable while nothing changes, so selecting it is cheap. */
export function useThreadSurfaces(threadId: string): ThreadSurfaces {
  return usePanelStore((state) => state.threads[threadId] ?? NO_SURFACES)
}
