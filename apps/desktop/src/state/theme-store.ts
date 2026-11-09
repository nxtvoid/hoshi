import { create } from 'zustand'
import { useSyncExternalStore } from 'react'
import { persist } from 'zustand/middleware'
import { invoke } from '@tauri-apps/api/core'
import {
  type Appearance,
  DEFAULT_APPEARANCE,
  normalizeAppearance,
  resolveAppearance,
  rootAppearance,
  type ThemePreference
} from './appearance'
import {
  chooseTheme,
  roleVariable,
  THEME_COLOR_ROLES,
  type ThemeAppearance,
  type ThemeColors,
  type ThemeDefinition,
  type ThemeSelection
} from './themes'

export type { ThemePreference } from './appearance'

type ThemeState = Appearance & {
  setPreference: (preference: ThemePreference) => void
  setTheme: (id: string) => void
  setSelection: (selection: ThemeSelection) => void
  setCustomThemes: (
    themes: readonly ThemeDefinition[],
    selection?: ThemeSelection
  ) => void
  update: (patch: Partial<Appearance>) => void
}

export const useThemeStore = create<ThemeState>()(
  persist(
    (set) => ({
      ...DEFAULT_APPEARANCE,
      setPreference: (preference) => set({ preference }),
      setTheme: (id) => set(chooseTheme(id)),
      setSelection: ({ theme, halves }) => set({ theme, halves }),
      setCustomThemes: (customThemes, selection) =>
        set(
          selection
            ? { customThemes, theme: selection.theme, halves: selection.halves }
            : { customThemes }
        ),
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

/** Colors the theme editor shows on the whole app while it's open. */
export type ThemePreview = { colors: ThemeColors; appearance: ThemeAppearance }

export const useThemePreviewStore = create<{ preview: ThemePreview | null }>(
  () => ({ preview: null })
)

export function setThemePreview(preview: ThemePreview | null) {
  useThemePreviewStore.setState({ preview })
}

const systemDark = window.matchMedia('(prefers-color-scheme: dark)')

function subscribeSystemDark(onChange: () => void) {
  systemDark.addEventListener('change', onChange)
  return () => systemDark.removeEventListener('change', onChange)
}

const readSystemDark = () => systemDark.matches

/** The appearance the app shows, after the mode and the themes' halves. */
export function useResolvedAppearance(): ThemeAppearance {
  const system = useSyncExternalStore(subscribeSystemDark, readSystemDark)
  const preview = useThemePreviewStore((state) => state.preview?.appearance)
  const resolved = useThemeStore((state) => resolveAppearance(state, system))
  return preview ?? resolved
}

export function useSystemDark() {
  return useSyncExternalStore(subscribeSystemDark, readSystemDark)
}

function setData(root: HTMLElement, key: string, value: string | undefined) {
  if (root.dataset[key] === value) return
  if (value === undefined) delete root.dataset[key]
  else root.dataset[key] = value
}

function setVariable(
  root: HTMLElement,
  name: string,
  value: string | undefined
) {
  if (root.style.getPropertyValue(name) === (value ?? '')) return
  if (value === undefined) root.style.removeProperty(name)
  else root.style.setProperty(name, value)
}

let appliedColors: ThemeColors | undefined | null = null

function setColors(root: HTMLElement, colors: ThemeColors | undefined) {
  if (colors === appliedColors) return
  appliedColors = colors
  for (const role of THEME_COLOR_ROLES) {
    if (colors) root.style.setProperty(roleVariable(role), colors[role])
    else root.style.removeProperty(roleVariable(role))
  }
  root.style.backgroundColor = colors?.background ?? ''
}

let mirrored: boolean | undefined

function applyAppearance() {
  const state = useThemeStore.getState()
  const preview = useThemePreviewStore.getState().preview
  const root = document.documentElement
  const applied = rootAppearance(state, systemDark.matches)
  const dark = preview ? preview.appearance === 'dark' : applied.dark
  root.classList.toggle('dark', dark)
  setData(root, 'palette', preview ? undefined : applied.palette)
  setColors(root, preview ? preview.colors : applied.colors)
  setData(root, 'font', applied.font)
  setData(root, 'codeFont', applied.codeFont)
  setData(root, 'composerFont', applied.composerFont)
  setData(root, 'terminalFont', applied.terminalFont)
  setData(root, 'fontSmoothing', applied.fontSmoothing)
  setData(root, 'textWeight', applied.textWeight)
  if (root.style.fontSize !== (applied.fontSize ?? '')) {
    root.style.fontSize = applied.fontSize ?? ''
  }
  setVariable(root, '--code-font-size', applied.codeFontSize)
  setVariable(root, '--composer-font-size', applied.composerFontSize)
  setVariable(root, '--terminal-font-size', applied.terminalFontSize)
  if (dark === mirrored) return
  mirrored = dark
  invoke('set_appearance', { dark }).catch(() => {})
}

export function startThemeSync() {
  applyAppearance()
  useThemeStore.subscribe(applyAppearance)
  useThemePreviewStore.subscribe(applyAppearance)
  systemDark.addEventListener('change', applyAppearance)
}
