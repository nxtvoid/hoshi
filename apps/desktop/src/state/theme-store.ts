import { create } from 'zustand'
import { useSyncExternalStore } from 'react'
import { persist } from 'zustand/middleware'
import { invoke } from '@tauri-apps/api/core'
import {
  type Appearance,
  DEFAULT_APPEARANCE,
  normalizeAppearance,
  resolveDark,
  rootAppearance,
  type ThemePreference
} from './appearance'

export type { ThemePreference } from './appearance'

type ThemeState = Appearance & {
  setPreference: (preference: ThemePreference) => void
  update: (patch: Partial<Appearance>) => void
}

export const useThemeStore = create<ThemeState>()(
  persist(
    (set) => ({
      ...DEFAULT_APPEARANCE,
      setPreference: (preference) => set({ preference }),
      update: (patch) => set(patch)
    }),
    {
      name: 'hoshi-theme',
      merge: (saved, current) => ({
        ...current,
        ...normalizeAppearance(saved)
      })
    }
  )
)

const systemDark = window.matchMedia('(prefers-color-scheme: dark)')

function subscribeSystemDark(onChange: () => void) {
  systemDark.addEventListener('change', onChange)
  return () => systemDark.removeEventListener('change', onChange)
}

const readSystemDark = () => systemDark.matches

export function useResolvedDark() {
  const preference = useThemeStore((state) => state.preference)
  const system = useSyncExternalStore(subscribeSystemDark, readSystemDark)
  return resolveDark(preference, system)
}

function setData(root: HTMLElement, key: string, value: string | undefined) {
  if (value === undefined) delete root.dataset[key]
  else root.dataset[key] = value
}

let mirrored: boolean | undefined

function applyAppearance() {
  const state = useThemeStore.getState()
  const root = document.documentElement
  const dark = resolveDark(state.preference, systemDark.matches)
  root.classList.toggle('dark', dark)
  const applied = rootAppearance(state)
  setData(root, 'palette', applied.palette)
  setData(root, 'font', applied.font)
  setData(root, 'codeFont', applied.codeFont)
  setData(root, 'textWeight', applied.textWeight)
  root.style.fontSize = applied.fontSize ?? ''
  if (applied.codeFontSize === undefined) {
    root.style.removeProperty('--code-font-size')
  } else {
    root.style.setProperty('--code-font-size', applied.codeFontSize)
  }
  if (dark === mirrored) return
  mirrored = dark
  invoke('set_appearance', { dark }).catch(() => {})
}

export function startThemeSync() {
  applyAppearance()
  useThemeStore.subscribe(applyAppearance)
  systemDark.addEventListener('change', applyAppearance)
}
