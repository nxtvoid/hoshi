import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import type { AppSettings } from '@/bindings/AppSettings'
import { useModelsStore } from './models-store'
import type { FavoriteModel } from '@/bindings/FavoriteModel'
import type { ClaudeCodeSettings } from '@/bindings/ClaudeCodeSettings'

type SettingsState = {
  settings: AppSettings | null
  error: 'load' | 'save' | null
  load: () => Promise<void>
  update: (settings: AppSettings) => Promise<void>
  updateSettings: (patch: Partial<AppSettings>) => Promise<void>
  updateClaudeCode: (patch: Partial<ClaudeCodeSettings>) => Promise<void>
  toggleFavoriteModel: (favorite: FavoriteModel) => Promise<void>
}

let loading: Promise<void> | null = null

export const useSettingsStore = create<SettingsState>()((set, get) => ({
  settings: null,
  error: null,
  load: () => {
    loading = (async () => {
      try {
        const settings = await invoke<AppSettings>('get_settings')
        set({ settings, error: null })
      } catch {
        set({ error: 'load' })
      }
    })()
    return loading
  },
  update: async (settings) => {
    const previous = get().settings
    set({ settings, error: null })
    try {
      await invoke<AppSettings>('update_settings', { settings })
      if (previous?.providers !== settings.providers)
        void useModelsStore.getState().load()
    } catch {
      set({ settings: previous, error: 'save' })
    }
  },
  updateSettings: async (patch) => {
    const current = get().settings
    if (!current) return
    await get().update({ ...current, ...patch })
  },
  toggleFavoriteModel: async (favorite) => {
    const current = get().settings
    if (!current) return
    const same = (candidate: FavoriteModel) =>
      candidate.provider === favorite.provider &&
      candidate.model === favorite.model
    const favoriteModels = current.favoriteModels.some(same)
      ? current.favoriteModels.filter((candidate) => !same(candidate))
      : [...current.favoriteModels, favorite]
    await get().update({ ...current, favoriteModels })
  },
  updateClaudeCode: async (patch) => {
    const current = get().settings
    if (!current) return
    await get().update({
      ...current,
      providers: {
        ...current.providers,
        claudeCode: { ...current.providers.claudeCode, ...patch }
      }
    })
  }
}))

/** The settings once they load, starting the load when nothing did yet; `null` if it failed. */
export async function loadedSettings(): Promise<AppSettings | null> {
  const { settings, load } = useSettingsStore.getState()
  if (settings) return settings
  await (loading ?? load())
  return useSettingsStore.getState().settings
}
