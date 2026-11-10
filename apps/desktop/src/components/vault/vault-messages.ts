import { m } from '@/paraglide/messages'
import type { VaultAccess } from '@/bindings/VaultAccess'
import type { CommandError } from '@/bindings/CommandError'

export const ACCESS_LABELS: Record<VaultAccess, () => string> = {
  ask: m.vault_access_ask,
  allow: m.vault_access_allow,
  hidden: m.vault_access_hidden
}

export const ACCESS_DESCRIPTIONS: Record<VaultAccess, () => string> = {
  ask: m.vault_access_ask_description,
  allow: m.vault_access_allow_description,
  hidden: m.vault_access_hidden_description
}

/** Why the vault refused, in the user's words. */
export function vaultErrorMessage(error: CommandError | null) {
  switch (error?.code) {
    case 'vaultSecretIncomplete':
      return m.vault_error_incomplete()
    case 'vaultUnavailable':
      return m.vault_error_unavailable({ detail: error.detail })
    case 'vaultSecretNotFound':
      return m.vault_error_not_found()
    case 'vaultValueMissing':
      return m.vault_error_value_missing()
    default:
      return m.error_generic()
  }
}
