import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import { useModelsStore } from './models-store'
import { useProviderUpdateStore } from './provider-update-store'
import type { ProviderAccountStatus } from '@/bindings/ProviderAccountStatus'

type ProviderAccountState = {
  /** `null` until checked once. */
  status: ProviderAccountStatus | null
  checking: boolean
  /** Runs the CLI, which takes a few seconds. */
  check: () => Promise<void>
}

export const useProviderAccountStore = create<ProviderAccountState>()(
  (set, get) => ({
    status: null,
    checking: false,
    check: async () => {
      if (get().checking) return
      const previous = get().status
      set({ checking: true })
      let status: ProviderAccountStatus = 'unknown'
      try {
        status = await invoke<ProviderAccountStatus>(
          'get_provider_account_status'
        )
      } catch (error) {
        console.error('hoshi: failed to check the provider account', error)
      }
      set({ status, checking: false })
      if (status !== 'ready') return
      if (previous === 'notInstalled') void useModelsStore.getState().load()
      const versions = useProviderUpdateStore.getState()
      if (previous === 'notInstalled' || !versions.version?.installed) {
        void versions.check()
      }
    }
  })
)
