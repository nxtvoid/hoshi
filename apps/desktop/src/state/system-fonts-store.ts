import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import type { SystemFont } from '@/bindings/SystemFont'

type SystemFontsState = {
  /** `null` until the first load finishes; kept for the rest of the session. */
  fonts: readonly SystemFont[] | null
}

export const useSystemFontsStore = create<SystemFontsState>()(() => ({
  fonts: null
}))

let loading: Promise<void> | null = null

export function loadSystemFonts(): void {
  if (loading || useSystemFontsStore.getState().fonts) return
  loading = invoke<SystemFont[]>('list_system_fonts')
    .then((fonts) => useSystemFontsStore.setState({ fonts }))
    .catch((error) => {
      console.error('hoshi: failed to list the installed fonts', error)
      useSystemFontsStore.setState({ fonts: [] })
    })
}
