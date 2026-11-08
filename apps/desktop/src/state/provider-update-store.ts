import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import { updateEnded } from '@/lib/updates'
import { useModelsStore } from './models-store'
import type { ProviderVersion } from '@/bindings/ProviderVersion'

type ProviderUpdateState = {
  /** `null` until checked, or while no provider is enabled. */
  version: ProviderVersion | null
  checking: boolean
  /** The user looked at how the last update ended. */
  seen: boolean
  load: () => Promise<void>
  /** Runs the CLI and looks up its latest release. */
  check: () => Promise<void>
  /** Starts the update; its progress arrives through `receive`. */
  update: () => Promise<void>
  markSeen: () => void
  receive: (version: ProviderVersion | null) => void
}

export const useProviderUpdateStore = create<ProviderUpdateState>()(
  (set, get) => ({
    version: null,
    checking: false,
    seen: true,
    load: async () => {
      try {
        const version = await invoke<ProviderVersion | null>(
          'get_provider_version'
        )
        if (version && !get().version) set({ version })
      } catch (error) {
        console.error('hoshi: failed to read the provider version', error)
      }
    },
    check: async () => {
      if (get().checking) return
      set({ checking: true })
      try {
        get().receive(
          await invoke<ProviderVersion | null>('check_provider_version')
        )
      } catch (error) {
        console.error('hoshi: failed to check the provider version', error)
      } finally {
        set({ checking: false })
      }
    },
    update: async () => {
      set({ seen: false })
      try {
        await invoke('update_provider')
      } catch (error) {
        console.error('hoshi: failed to start the provider update', error)
      }
    },
    markSeen: () => set({ seen: true }),
    receive: (version) => {
      const previous = get().version?.update
      if (version && updateEnded(previous, version.update)) {
        set({ seen: false })
        if (version.update.status === 'succeeded') {
          void useModelsStore.getState().load()
        }
      }
      set({ version })
    }
  })
)
