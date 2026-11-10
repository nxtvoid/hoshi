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
  showNote,
  showTurnDiff,
  type Surface,
  type SurfaceKind,
  type ThreadSurfaces,
  updateSurface
} from './panel-surfaces'

export const PANEL_WIDTH = { min: 320, max: 760, default: 440 } as const

type PanelState = {
  /** The threads whose panel is open: each thread keeps its own, with its surfaces. */
  opened: Record<string, true>
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
  /** Opens the panel on what a turn changed, at `focusPath` when given. */
  openTurnDiff: (threadId: string, turnId: string, focusPath?: string) => void
  /** Opens the panel on a note as the message `itemId` carried it. */
  openNote: (threadId: string, itemId: string, noteId: string) => void
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
      const show = (threadId: string) =>
        set((state) =>
          state.opened[threadId]
            ? state
            : { opened: { ...state.opened, [threadId]: true } }
        )
      const hide = (threadId: string) =>
        set((state) => ({
          opened: withoutKeys(state.opened, [threadId]),
          maximized: false
        }))
      return {
        opened: {},
        detailsOpen: false,
        maximized: false,
        width: PANEL_WIDTH.default,
        threads: {},
        toggle: (threadId) =>
          get().opened[threadId] ? hide(threadId) : show(threadId),
        setDetailsOpen: (detailsOpen) => set({ detailsOpen }),
        close: hide,
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
          show(threadId)
          const activeId = get().threads[threadId]?.activeId
          if (kind === 'terminal' && activeId) {
            useTerminalStore.getState().requestFocus(activeId)
          }
        },
        openTurnDiff: (threadId, turnId, focusPath) => {
          changeThread(threadId, (panel) =>
            showTurnDiff(panel, turnId, focusPath, makeId)
          )
          show(threadId)
        },
        openNote: (threadId, itemId, noteId) => {
          changeThread(threadId, (panel) =>
            showNote(panel, itemId, noteId, makeId)
          )
          show(threadId)
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
          if (get().threads[threadId]?.surfaces.length === 0) hide(threadId)
        },
        updateSurface: (threadId, surfaceId, change) =>
          changeThread(threadId, (panel) =>
            updateSurface(panel, surfaceId, change)
          ),
        forget: (threadIds) =>
          set((state) => {
            const threads = withoutKeys(state.threads, threadIds)
            const opened = withoutKeys(state.opened, threadIds)
            return threads === state.threads && opened === state.opened
              ? state
              : { threads, opened }
          })
      }
    },
    {
      name: 'hoshi-panel',
      partialize: ({ opened, detailsOpen, maximized, width, threads }) => ({
        opened,
        detailsOpen,
        maximized,
        width,
        threads
      }),
      merge: (saved, current) => {
        const kept = { ...current, ...(saved as Partial<PanelState>) }
        const threads = Object.fromEntries(
          Object.entries(kept.threads).map(([threadId, panel]) => [
            threadId,
            knownSurfaces(panel)
          ])
        )
        return { ...kept, threads }
      }
    }
  )
)

/** Whether the panel is open in the thread; never with no thread. */
export function usePanelOpen(threadId: string | undefined): boolean {
  return usePanelStore((state) =>
    threadId ? state.opened[threadId] === true : false
  )
}

/** The surfaces of a thread; stable while nothing changes, so selecting it is cheap. */
export function useThreadSurfaces(threadId: string): ThreadSurfaces {
  return usePanelStore((state) => state.threads[threadId] ?? NO_SURFACES)
}
