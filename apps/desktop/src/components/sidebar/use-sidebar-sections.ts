import { useMemo } from 'react'
import { useAppStore } from '@/state/app-store'
import { selectQuietThreads } from '@/state/quiet-threads'
import type { Thread } from '@/bindings/Thread'
import { useUnseenStore } from '@/state/unseen-store'
import { useSidebarStore } from '@/state/sidebar-store'
import type { ForgeAccount } from '@/bindings/ForgeAccount'
import type { PullRequest } from '@/bindings/PullRequest'
import { useSettingsStore } from '@/state/settings-store'
import { useForgeAccountsStore } from '@/state/forge-accounts-store'
import {
  type ForgeLogins,
  inactiveAfterMs,
  inboxCount,
  inboxSections,
  newestFirst,
  type SidebarQuery,
  sidebarSections,
  withDelegatedWork
} from '@/state/thread-sections'

type SidebarInputs = {
  threads: Thread[]
  pullRequests: Record<string, PullRequest>
  query: SidebarQuery
  unseen: Record<string, true>
  held: string | null
}

function loginsOf(accounts: ForgeAccount[] | null): ForgeLogins {
  const logins: ForgeLogins = {}
  for (const account of accounts ?? []) {
    if (account.login) logins[account.forge] = account.login
  }
  return logins
}

function useLogins() {
  const accounts = useForgeAccountsStore((state) => state.accounts)
  return useMemo(() => loginsOf(accounts), [accounts])
}

function sectionsOf({
  threads,
  pullRequests,
  query,
  unseen,
  held
}: SidebarInputs) {
  return sidebarSections(
    threads,
    pullRequests,
    query,
    Date.now(),
    held ? { ...unseen, [held]: true } : unseen
  )
}

function inboxOf(
  threads: Thread[],
  pullRequests: Record<string, PullRequest>,
  unseen: Record<string, true>,
  logins: ForgeLogins
) {
  const sorted = withDelegatedWork(threads).sort((a, b) =>
    newestFirst(a.updatedAt, b.updatedAt)
  )
  return inboxSections(sorted, pullRequests, Date.now(), unseen, logins)
}

/** The sections the sidebar lists right now, for its view, search, filter and sort. */
export function useSidebarSections() {
  const threads = useAppStore(selectQuietThreads)
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
  const logins = useLogins()

  return useMemo(
    () =>
      sectionsOf({
        threads,
        pullRequests,
        query: {
          view,
          projectId,
          search,
          filter,
          sort,
          inactiveAfterMs: inactiveAfterMs(inactiveDays),
          logins
        },
        unseen,
        held
      }),
    [
      threads,
      pullRequests,
      view,
      projectId,
      search,
      filter,
      sort,
      inactiveDays,
      logins,
      unseen,
      held
    ]
  )
}

/** The sidebar's sections as they are now, for a handler that runs outside rendering. */
export function currentSidebarSections() {
  const { pullRequests, search, selectedProjectId } = useAppStore.getState()
  const threads = selectQuietThreads(useAppStore.getState())
  const { unseen, held } = useUnseenStore.getState()
  const { view, filter, threadSort } = useSidebarStore.getState()
  const inactiveDays =
    useSettingsStore.getState().settings?.general.inactiveAfterDays
  return sectionsOf({
    threads,
    pullRequests,
    query: {
      view,
      projectId: selectedProjectId,
      search,
      filter,
      sort: threadSort,
      inactiveAfterMs: inactiveAfterMs(inactiveDays),
      logins: loginsOf(useForgeAccountsStore.getState().accounts)
    },
    unseen,
    held
  })
}

/** The inbox across every project, newest first, whatever the sidebar shows. */
function useInboxSections() {
  const threads = useAppStore(selectQuietThreads)
  const pullRequests = useAppStore((state) => state.pullRequests)
  const unseen = useUnseenStore((state) => state.unseen)
  const logins = useLogins()

  return useMemo(
    () => inboxOf(threads, pullRequests, unseen, logins),
    [threads, pullRequests, unseen, logins]
  )
}

/** Threads waiting on the user in every project, as they are now, for the jump shortcut. */
export function currentAttentionThreads(): Thread[] {
  const { threads, pullRequests } = useAppStore.getState()
  const { unseen } = useUnseenStore.getState()
  const logins = loginsOf(useForgeAccountsStore.getState().accounts)
  return (
    inboxOf(threads, pullRequests, unseen, logins).find(
      (section) => section.id === 'attention'
    )?.threads ?? []
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
