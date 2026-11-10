import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import type { VaultSecret } from '@/bindings/VaultSecret'
import type { VaultSecretInput } from '@/bindings/VaultSecretInput'

type VaultState = {
  /** The vault's secrets by name, without values; `null` until first read. */
  secrets: VaultSecret[] | null
  load: () => Promise<void>
  /** Adds a secret, or changes the one with `secretId`. Rejects with a `CommandError`. */
  save: (
    secretId: string | null,
    secret: VaultSecretInput
  ) => Promise<VaultSecret>
  remove: (secretId: string) => Promise<void>
}

export const useVaultStore = create<VaultState>()((set, get) => ({
  secrets: null,
  load: async () => {
    try {
      set({ secrets: await invoke<VaultSecret[]>('list_vault_secrets') })
    } catch (error) {
      console.error('hoshi: failed to read the vault', error)
      if (get().secrets === null) set({ secrets: [] })
    }
  },
  save: async (secretId, secret) => {
    const saved = await invoke<VaultSecret>('save_vault_secret', {
      secretId,
      secret
    })
    await get().load()
    return saved
  },
  remove: async (secretId) => {
    await invoke('delete_vault_secret', { secretId })
    await get().load()
  }
}))

/** Copies the value of one of a secret's fields to the clipboard; it never reaches the page. */
export function copySecret(secretId: string, fieldId: string) {
  return invoke<void>('copy_vault_secret', { secretId, fieldId })
}

/** Reads the vault the first time something shows it. */
export function ensureVaultLoaded() {
  const { secrets, load } = useVaultStore.getState()
  if (secrets === null) void load()
}
