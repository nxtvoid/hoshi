import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import { invoke } from '@tauri-apps/api/core'
import {
  activateTerminal,
  addTerminal,
  closeTerminal,
  NO_DRAWER,
  showTerminal,
  type TerminalDrawer,
  toggleDrawer
} from './terminal-drawer'

export const DRAWER_HEIGHT = { min: 120, max: 800, default: 280 } as const

type TerminalState = {
  height: number
  /** Drawers by thread id. */
  drawers: Record<string, TerminalDrawer>
  /** The terminal that takes the keyboard once its view is ready. */
  pendingFocus: string | null
  setHeight: (height: number) => void
  toggleDrawer: (threadId: string) => void
  addTerminal: (threadId: string) => void
  activateTerminal: (threadId: string, terminalId: string) => void
  /** Opens the drawer on a terminal the engine started, such as a worktree's setup. */
  showTerminal: (threadId: string, terminalId: string) => void
  /** Stops the terminal's shell too. */
  closeTerminal: (threadId: string, terminalId: string) => void
  requestFocus: (terminalId: string) => void
  clearFocus: () => void
}

const makeId = () => crypto.randomUUID()

export const useTerminalStore = create<TerminalState>()(
  persist(
    (set, get) => {
      const changeDrawer = (
        threadId: string,
        change: (drawer: TerminalDrawer) => TerminalDrawer
      ) => {
        const drawer = change(get().drawers[threadId] ?? NO_DRAWER)
        set((state) => ({
          drawers: { ...state.drawers, [threadId]: drawer },
          pendingFocus: drawer.open ? drawer.activeId : state.pendingFocus
        }))
      }
      return {
        height: DRAWER_HEIGHT.default,
        drawers: {},
        pendingFocus: null,
        setHeight: (height) =>
          set({
            height: Math.round(
              Math.min(DRAWER_HEIGHT.max, Math.max(DRAWER_HEIGHT.min, height))
            )
          }),
        toggleDrawer: (threadId) =>
          changeDrawer(threadId, (drawer) => toggleDrawer(drawer, makeId)),
        addTerminal: (threadId) =>
          changeDrawer(threadId, (drawer) => addTerminal(drawer, makeId)),
        activateTerminal: (threadId, terminalId) =>
          changeDrawer(threadId, (drawer) =>
            activateTerminal(drawer, terminalId)
          ),
        showTerminal: (threadId, terminalId) =>
          changeDrawer(threadId, (drawer) => showTerminal(drawer, terminalId)),
        closeTerminal: (threadId, terminalId) => {
          void invoke('close_terminal', { terminalId })
          changeDrawer(threadId, (drawer) => closeTerminal(drawer, terminalId))
        },
        requestFocus: (terminalId) => set({ pendingFocus: terminalId }),
        clearFocus: () => set({ pendingFocus: null })
      }
    },
    {
      name: 'hoshi-terminals',
      partialize: ({ height, drawers }) => ({ height, drawers })
    }
  )
)

export function useTerminalDrawer(threadId: string): TerminalDrawer {
  return useTerminalStore((state) => state.drawers[threadId] ?? NO_DRAWER)
}
