import { useMemo } from 'react'
import { useAppStore } from '@/state/app-store'
import { useRequestStore } from '@/state/request-store'
import { type BusyThreads, busyThreads } from '@/lib/agents'
import { useBackgroundTaskStore } from '@/state/background-task-store'

/** The threads at rest that still have something going; the same object until they change. */
export function useBusyThreads(): BusyThreads {
  const pendingTasks = useRequestStore((state) => state.pendingTasks)
  const inBackground = useBackgroundTaskStore((state) => state.byThread)
  const ids = useAppStore((state) =>
    Object.keys(busyThreads(state.threads, pendingTasks, inBackground))
      .sort()
      .join(' ')
  )
  return useMemo(
    () => Object.fromEntries(ids.split(' ').map((id) => [id, true])),
    [ids]
  )
}
