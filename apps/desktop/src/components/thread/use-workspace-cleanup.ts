import { useState } from 'react'
import { m } from '@/paraglide/messages'
import { invoke } from '@tauri-apps/api/core'
import { notify } from '@/state/dialog-store'
import type { CommandError } from '@/bindings/CommandError'

export function cleanupErrorMessage(error: CommandError): string {
  switch (error.code) {
    case 'worktreeDirty':
      return m.cleanup_error_dirty()
    case 'terminalsOpen':
      return m.cleanup_error_terminals()
    case 'pullRequestNotMerged':
      return m.cleanup_error_not_merged()
    case 'unpushedCommits':
      return m.cleanup_error_unpushed()
    case 'worktreeLocked':
      return error.reason
        ? m.cleanup_error_locked_reason({ reason: error.reason })
        : m.cleanup_error_locked()
    case 'nothingToClean':
      return m.cleanup_error_nothing()
    case 'worktreeAdopted':
      return m.cleanup_error_adopted()
    case 'threadBusy':
      return m.error_thread_busy()
    case 'gitFailed':
      return error.detail
    default:
      return m.error_generic()
  }
}

export type CleanupKind = 'release' | 'cleanUp'

const COMMANDS: Record<CleanupKind, string> = {
  release: 'release_worktree',
  cleanUp: 'clean_up_thread'
}

/** Runs one of them; resolves to the error, or `null` when it went through. */
export async function runCleanup(
  kind: CleanupKind,
  threadId: string
): Promise<CommandError | null> {
  try {
    await invoke(COMMANDS[kind], { threadId })
    return null
  } catch (error) {
    return error as CommandError
  }
}

/** The same, telling about a refusal in a dialog, for places with no room to show it. */
export async function runCleanupWithDialog(
  kind: CleanupKind,
  threadId: string
) {
  const error = await runCleanup(kind, threadId)
  if (error) {
    await notify({
      title: kind === 'release' ? m.release_worktree() : m.clean_up_thread(),
      description: cleanupErrorMessage(error),
      confirmLabel: m.dialog_ok(),
      tone: 'warning'
    })
  }
}

/** Removing a thread's worktree, and its branch once merged, with the refusal kept to show. */
export function useWorkspaceCleanup(threadId: string) {
  const [pending, setPending] = useState<CleanupKind | null>(null)
  const [error, setError] = useState<CommandError | null>(null)

  const run = async (kind: CleanupKind) => {
    setPending(kind)
    setError(null)
    const failure = await runCleanup(kind, threadId)
    setError(failure)
    setPending(null)
    return failure === null
  }

  return { run, pending, error }
}
