import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import type { ForgeKind } from '@/bindings/ForgeKind'
import type { ForgeAccount } from '@/bindings/ForgeAccount'

type ForgeAccountsState = {
  /** `null` until the forges first answer. */
  accounts: ForgeAccount[] | null
  /** Asks the forges again; the last answer stays shown meanwhile. */
  load: () => Promise<void>
}

export const useForgeAccountsStore = create<ForgeAccountsState>()((set) => ({
  accounts: null,
  load: async () => {
    try {
      set({ accounts: await invoke<ForgeAccount[]>('get_forge_accounts') })
    } catch (error) {
      console.error('hoshi: forge accounts failed', error)
    }
  }
}))

/** `undefined` while unknown, `null` when signed out. */
export const forgeLogin = (
  accounts: ForgeAccount[] | null,
  forge: ForgeKind
): string | null | undefined =>
  accounts === null
    ? undefined
    : (accounts.find((account) => account.forge === forge)?.login ?? null)
