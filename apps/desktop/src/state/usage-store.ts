import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import type { UsageLimits } from '@/bindings/UsageLimits'

type UsageState = {
  /** `null` until the provider was read once, in this run or an earlier one. */
  limits: UsageLimits | null
  checking: boolean
  /** The last time the provider was asked, it couldn't say. */
  failed: boolean
  /** The limits last read, without asking the provider. */
  load: () => Promise<void>
  /** Asks the provider, which answers at once if it did within the last minute. */
  refresh: (options?: { force?: boolean }) => Promise<void>
  /** The engine sends every window it knows, merged with what turns reported. */
  receive: (limits: UsageLimits) => void
}

export const useUsageStore = create<UsageState>()((set, get) => ({
  limits: null,
  checking: false,
  failed: false,
  load: async () => {
    try {
      const known = await invoke<UsageLimits | null>('get_known_usage_limits')
      if (known && !get().limits) set({ limits: known })
    } catch (error) {
      console.error('hoshi: failed to read the known usage limits', error)
    }
  },
  refresh: async ({ force = false } = {}) => {
    if (get().checking) return
    set({ checking: true })
    try {
      if (!get().limits) await get().load()
      const limits = await invoke<UsageLimits | null>('get_usage_limits', {
        force
      })
      set(limits ? { limits, failed: false } : { failed: true })
    } catch (error) {
      console.error('hoshi: failed to read usage limits', error)
      set({ failed: true })
    } finally {
      set({ checking: false })
    }
  },
  receive: (limits) => set({ limits, failed: false })
}))
