import { withCleanup } from '@hoshi/utils'
import { useState } from 'react'
import { m } from '@/paraglide/messages'
import { invoke } from '@tauri-apps/api/core'
import type { CommandError } from '@/bindings/CommandError'
import type { PullRequestAction } from '@/bindings/PullRequestAction'

export function forgeErrorMessage(error: CommandError): string {
  switch (error.code) {
    case 'forgeUnauthorized':
      return m.forge_error_unauthorized()
    case 'forgeRateLimited':
      return m.forge_error_rate_limited()
    case 'forgeRejected':
      return error.detail
    case 'forgeUnreachable':
      return m.forge_error_unreachable()
    default:
      return m.forge_error_unavailable()
  }
}

export type PullRequestActions = {
  /** Resolves to whether the forge made the change. */
  run: (action: PullRequestAction) => Promise<boolean>
  pending: boolean
  error: string | null
  clearError: () => void
}

/** Runs changes on a thread's pull request and reloads its detail after each one. */
export function usePullRequestActions(
  threadId: string,
  forgeId: string,
  reload: () => void
): PullRequestActions {
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const run = async (action: PullRequestAction) => {
    setPending(true)
    setError(null)
    return await withCleanup(
      async () => {
        try {
          await invoke('run_pull_request_action', { threadId, forgeId, action })
          reload()
          return true
        } catch (failure) {
          setError(forgeErrorMessage(failure as CommandError))
          return false
        }
      },
      () => {
        setPending(false)
      }
    )
  }

  return { run, pending, error, clearError: () => setError(null) }
}
