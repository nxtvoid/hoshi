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

const sameAccounts = (a: ForgeAccount[] | null, b: ForgeAccount[]) =>
  a !== null &&
  a.length === b.length &&
  a.every((account) =>
    b.some(
      (other) => other.forge === account.forge && other.login === account.login
    )
  )

/** The window coming back reads the accounts again, at most this often: they may have changed outside hoshi. */
const RECHECK_AFTER_MS = 30_000

export const useForgeAccountsStore = create<ForgeAccountsState>()(
  (set, get) => ({
    accounts: null,
    load: async () => {
      try {
        const accounts = await invoke<ForgeAccount[]>('get_forge_accounts')
        if (!sameAccounts(get().accounts, accounts)) set({ accounts })
      } catch (error) {
        console.error('hoshi: forge accounts failed', error)
      }
    }
  })
)

/** Reads the accounts now, and again when the window comes back after a while. */
export function watchForgeAccounts() {
  let checkedAt = 0
  const load = () => {
    checkedAt = Date.now()
    void useForgeAccountsStore.getState().load()
  }
  const onFocus = () => {
    if (Date.now() - checkedAt >= RECHECK_AFTER_MS) load()
  }
  load()
  window.addEventListener('focus', onFocus)
  return () => window.removeEventListener('focus', onFocus)
}

/** `undefined` while unknown, `null` when signed out. */
export const forgeLogin = (
  accounts: ForgeAccount[] | null,
  forge: ForgeKind
): string | null | undefined =>
  accounts === null
    ? undefined
    : (accounts.find((account) => account.forge === forge)?.login ?? null)
