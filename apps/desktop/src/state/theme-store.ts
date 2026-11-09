import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import { invoke } from '@tauri-apps/api/core'

export type ThemePreference = 'system' | 'light' | 'dark'

type ThemeState = {
  preference: ThemePreference
  setPreference: (preference: ThemePreference) => void
}

export const useThemeStore = create<ThemeState>()(
  persist(
    (set) => ({
      preference: 'system',
      setPreference: (preference) => set({ preference })
    }),
    { name: 'hoshi-theme' }
  )
)

const systemDark = window.matchMedia('(prefers-color-scheme: dark)')

let mirrored: boolean | undefined

function applyTheme() {
  const { preference } = useThemeStore.getState()
  const dark =
    preference === 'dark' || (preference === 'system' && systemDark.matches)
  document.documentElement.classList.toggle('dark', dark)
  if (dark === mirrored) return
  mirrored = dark
  invoke('set_appearance', { dark }).catch(() => {})
}

export function startThemeSync() {
  applyTheme()
  useThemeStore.subscribe(applyTheme)
  systemDark.addEventListener('change', applyTheme)
}
