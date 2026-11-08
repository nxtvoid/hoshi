import { useMemo } from 'react'
import { useAppStore } from '@/state/app-store'
import type { Thread } from '@/bindings/Thread'
import { useUnseenStore } from '@/state/unseen-store'
import { useSidebarStore } from '@/state/sidebar-store'
import { useSettingsStore } from '@/state/settings-store'
import {
  inactiveAfterMs,
  inboxCount,
  inboxSections,
  sidebarSections
} from '@/state/thread-sections'

/** The sections the sidebar lists right now, for its view, search, filter and sort. */
export function useSidebarSections() {
  const threads = useAppStore((state) => state.threads)
  const pullRequests = useAppStore((state) => state.pullRequests)
  const search = useAppStore((state) => state.search)
  const projectId = useAppStore((state) => state.selectedProjectId)
  const unseen = useUnseenStore((state) => state.unseen)
  const held = useUnseenStore((state) => state.held)
  const view = useSidebarStore((state) => state.view)
  const filter = useSidebarStore((state) => state.filter)
  const sort = useSidebarStore((state) => state.threadSort)
  const inactiveDays = useSettingsStore(
    (state) => state.settings?.general.inactiveAfterDays
  )

  return useMemo(
    () =>
      sidebarSections(
        threads,
        pullRequests,
        {
          view,
          projectId,
          search,
          filter,
          sort,
          inactiveAfterMs: inactiveAfterMs(inactiveDays)
        },
        Date.now(),
        held ? { ...unseen, [held]: true } : unseen
      ),
    [
      threads,
      pullRequests,
      view,
      projectId,
      search,
      filter,
      sort,
      inactiveDays,
      unseen,
      held
    ]
  )
}

/** The inbox across every project, newest first, whatever the sidebar shows. */
function useInboxSections() {
  const threads = useAppStore((state) => state.threads)
  const pullRequests = useAppStore((state) => state.pullRequests)
  const unseen = useUnseenStore((state) => state.unseen)

  return useMemo(() => {
    const sorted = [...threads].sort((a, b) =>
      b.updatedAt.localeCompare(a.updatedAt)
    )
    return inboxSections(sorted, pullRequests, Date.now(), unseen)
  }, [threads, pullRequests, unseen])
}

const NO_THREADS: Thread[] = []

/** Threads waiting on the user in every project, for the jump shortcut. */
export function useAttentionThreads() {
  const sections = useInboxSections()
  return (
    sections.find((section) => section.id === 'attention')?.threads ??
    NO_THREADS
  )
}

/** What the inbox badge shows, and whether any of it waits on the user. */
export function useInboxBadge() {
  const sections = useInboxSections()
  return {
    count: inboxCount(sections),
    waiting: sections.some((section) => section.id === 'attention')
  }
}
