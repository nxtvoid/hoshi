import type { VaultSecret } from '@/bindings/VaultSecret'
import type { VaultAccess } from '@/bindings/VaultAccess'

export const VAULT_ACCESSES: VaultAccess[] = ['ask', 'allow', 'hidden']

/**
 * The secrets a thread of `projectId` can be answered with: its project's first, then the
 * ones of every project. A chat reaches only the latter.
 */
export function secretsInReach(
  secrets: readonly VaultSecret[],
  projectId: string | null
): VaultSecret[] {
  const own = secrets.filter(
    (secret) => secret.projectId !== null && secret.projectId === projectId
  )
  const general = secrets.filter((secret) => secret.projectId === null)
  return [...own, ...general]
}

/**
 * The secrets that can answer a request for `values` values in a thread of `projectId`:
 * the ones in its reach that hold as many.
 */
export function secretsAnswering(
  secrets: readonly VaultSecret[],
  projectId: string | null,
  values: number
): VaultSecret[] {
  return secretsInReach(secrets, projectId).filter(
    (secret) => secret.fields.length === values
  )
}

/** What a secret's values are called; none when it is a single unnamed value. */
export function fieldLabels(secret: VaultSecret): string[] {
  return secret.fields.map((field) => field.label).filter((label) => label)
}

/**
 * How an agent gets a secret, given the setting that lets agents take any they see: a
 * secret that asks is then taken freely too.
 */
export function effectiveAccess(
  access: VaultAccess,
  alwaysAllow: boolean
): VaultAccess {
  return access === 'ask' && alwaysAllow ? 'allow' : access
}

/** What a secret's access can be set to: asking first means nothing once agents take any. */
export function accessChoices(alwaysAllow: boolean): VaultAccess[] {
  return alwaysAllow
    ? VAULT_ACCESSES.filter((access) => access !== 'ask')
    : VAULT_ACCESSES
}
