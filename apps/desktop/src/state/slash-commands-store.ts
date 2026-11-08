import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import type { SlashCommand } from '@/bindings/SlashCommand'

/** Commands differ per folder, so they are kept per thread, or per project for a draft; a `null` project is a chat. */
export const slashCommandsKey = (
  projectId: string | null,
  threadId: string | null
) => `${projectId ?? ''}:${threadId ?? ''}`

type SlashCommandsState = {
  byKey: Record<string, SlashCommand[]>
  loading: Record<string, boolean>
  /** Asks again each time; the engine answers from its cache while it is fresh. */
  load: (projectId: string | null, threadId: string | null) => Promise<void>
}

export const useSlashCommandsStore = create<SlashCommandsState>()(
  (set, get) => ({
    byKey: {},
    loading: {},
    load: async (projectId, threadId) => {
      const key = slashCommandsKey(projectId, threadId)
      if (get().loading[key]) return
      set((state) => ({ loading: { ...state.loading, [key]: true } }))
      try {
        const commands = await invoke<SlashCommand[] | null>(
          'get_slash_commands',
          { projectId, threadId }
        )
        if (commands) {
          set((state) => ({ byKey: { ...state.byKey, [key]: commands } }))
        }
      } catch (error) {
        console.error('hoshi: failed to load slash commands', error)
      } finally {
        set((state) => ({ loading: { ...state.loading, [key]: false } }))
      }
    }
  })
)
