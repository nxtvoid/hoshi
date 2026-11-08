import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import type { ProviderModels } from '@/bindings/ProviderModels'

type ModelsState = {
  /** `null` while loading or while no provider is enabled. */
  catalog: ProviderModels | null
  load: () => Promise<void>
}

export const useModelsStore = create<ModelsState>()((set) => ({
  catalog: null,
  load: async () => {
    try {
      set({ catalog: await invoke<ProviderModels | null>('get_models') })
    } catch (error) {
      console.error('hoshi: failed to load models', error)
    }
  }
}))
