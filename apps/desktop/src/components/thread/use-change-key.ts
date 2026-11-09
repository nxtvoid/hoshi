import { useEffect, useState } from 'react'
import { useAppStore } from '@/state/app-store'

/** While a turn adds items, the checkout is read again at most this often. */
const ACTIVITY_MS = 1000

const activityOf = (threadId: string) =>
  useAppStore.getState().threads.find((thread) => thread.id === threadId)
    ?.updatedAt ?? ''

/**
 * Changes when a thread's checkout may have changed: at once when its branch, worktree or
 * status changes, as when a turn ends, and at most once a second while its agent works.
 * Empty while the thread is unknown.
 */
export function useChangeKey(threadId: string) {
  const checkout = useAppStore((state) => {
    const thread = state.threads.find((candidate) => candidate.id === threadId)
    return thread
      ? `${thread.branch}|${thread.worktreePath}|${thread.status}`
      : null
  })
  const activity = useAppStore(
    (state) =>
      state.threads.find((candidate) => candidate.id === threadId)?.updatedAt ??
      ''
  )
  const [shown, setShown] = useState({ threadId, checkout, activity })
  const current = shown.threadId === threadId && shown.checkout === checkout
  if (!current) setShown({ threadId, checkout, activity })
  const behind = current && shown.activity !== activity
  const since = shown.activity

  useEffect(() => {
    if (!behind) return
    const timer = setTimeout(
      () =>
        setShown((value) =>
          value.activity === since
            ? { ...value, activity: activityOf(threadId) }
            : value
        ),
      ACTIVITY_MS
    )
    return () => clearTimeout(timer)
  }, [behind, since, threadId])

  return checkout === null ? '' : `${checkout}|${shown.activity}`
}
