import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import type { AppSettings } from '@/bindings/AppSettings'
import type { ClaudeCodeSettings } from '@/bindings/ClaudeCodeSettings'

type SettingsState = {
  settings: AppSettings | null
  error: 'load' | 'save' | null
  load: () => Promise<void>
  update: (settings: AppSettings) => Promise<void>
  updateClaudeCode: (patch: Partial<ClaudeCodeSettings>) => Promise<void>
}

export const useSettingsStore = create<SettingsState>()((set, get) => ({
  settings: null,
  error: null,
  load: async () => {
    try {
      const settings = await invoke<AppSettings>('get_settings')
      set({ settings, error: null })
    } catch {
      set({ error: 'load' })
    }
  },
  update: async (settings) => {
    const previous = get().settings
    set({ settings, error: null })
    try {
      await invoke<AppSettings>('update_settings', { settings })
    } catch {
      set({ settings: previous, error: 'save' })
    }
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
