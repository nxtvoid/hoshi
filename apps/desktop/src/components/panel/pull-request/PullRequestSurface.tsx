import { cn } from '@hoshi/utils'
import { CodeTab } from './CodeTab'
import { SummaryTab } from './SummaryTab'
import { TimelineTab } from './TimelineTab'
import { m } from '@/paraglide/messages'
import type * as React from 'react'
import { useEffect, useState } from 'react'
import { useAppStore } from '@/state/app-store'
import { usePullRequestStore } from '@/state/pull-request-store'
import { HugeiconsIcon } from '@hugeicons/react'
import { PanelMessage } from '../ChangedFiles'
import { openUrl } from '@tauri-apps/plugin-opener'
import { PullRequestHeader } from './PullRequestHeader'
import { CHECK_STATE } from './shared'
import { Cancel01Icon } from '@hugeicons/core-free-icons'
import { usePullRequestActions } from './use-pull-request-actions'
import type { ChecksState } from '@/bindings/ChecksState'
import type { PullRequestCheck } from '@/bindings/PullRequestCheck'
import {
  Popover,
  PopoverContent,
  PopoverTrigger
} from '@hoshi/ui/components/popover'

type Tab = 'summary' | 'timeline' | 'code'

const TABS: { id: Tab; label: () => string }[] = [
  { id: 'summary', label: m.pr_tab_summary },
  { id: 'timeline', label: m.pr_tab_timeline },
  { id: 'code', label: m.pr_tab_code }
]

/** A thread's pull request, laid out like the forge's own page, with its actions. */
export function PullRequestSurface({ threadId }: { threadId: string }) {
  const pullRequest = useAppStore((state) => state.pullRequests[threadId])
  const [tab, setTab] = useState<Tab>('summary')
  /** Tabs stay mounted once opened, so going back to one does not read it again. */
  const [visited, setVisited] = useState<ReadonlySet<Tab>>(new Set(['summary']))
  const ready = usePullRequestStore((state) => state.details[threadId] ?? null)
  const failed = usePullRequestStore((state) => state.failed[threadId] === true)
  const load = usePullRequestStore((state) => state.load)
  const prefetch = usePullRequestStore((state) => state.prefetch)
  const version = pullRequest
    ? `${pullRequest.number}|${pullRequest.updatedAt}|${pullRequest.state}|${pullRequest.checks}|${pullRequest.unresolvedComments}`
    : null
  const actions = usePullRequestActions(
    threadId,
    ready?.forgeId ?? '',
    () => void load(threadId)
  )

  // Shows the cached detail at once; the engine reads a fresh one when it is old, and
  // the files start loading now so the Code tab opens ready.
  useEffect(() => {
    if (version === null) return
    void load(threadId)
    prefetch(threadId)
  }, [threadId, version, load, prefetch])

  const selectTab = (next: Tab) => {
    setTab(next)
    setVisited((current) => new Set(current).add(next))
  }

  if (!pullRequest) return <PanelMessage>{m.no_pull_request()}</PanelMessage>
  const canComment =
    ready !== null &&
    (pullRequest.state === 'open' || pullRequest.state === 'draft')

  return (
    <div className='flex min-h-0 flex-1 flex-col'>
      <PullRequestHeader
        pullRequest={pullRequest}
        detail={ready}
        actions={actions}
      />
      {actions.error ? (
        <div
          role='alert'
          className='border-border bg-destructive/10 text-destructive flex items-start gap-2 border-b px-3 py-2 text-xs'
        >
          <span className='min-w-0 flex-1'>{actions.error}</span>
          <button
            type='button'
            aria-label={m.dismiss()}
            onClick={actions.clearError}
            className='hover:text-foreground focus-visible:ring-ring rounded outline-none focus-visible:ring-2'
          >
            <HugeiconsIcon
              icon={Cancel01Icon}
              strokeWidth={2}
              className='size-3.5'
            />
          </button>
        </div>
      ) : null}
      <div className='border-border flex shrink-0 items-center gap-1 border-b px-3 py-1.5'>
        <div role='tablist' className='bg-muted flex rounded-lg p-0.5'>
          {TABS.map(({ id, label }) => (
            <button
              key={id}
              type='button'
              role='tab'
              aria-selected={tab === id}
              onClick={() => selectTab(id)}
              className={cn(
                'focus-visible:ring-ring rounded-md px-2.5 py-1 text-xs font-medium outline-none focus-visible:ring-2',
                tab === id
                  ? 'bg-background text-foreground shadow-xs'
                  : 'text-muted-foreground hover:text-foreground'
              )}
            >
              {label()}
            </button>
          ))}
        </div>
        {ready ? <ChecksSummary checks={ready.checks} /> : null}
      </div>
      {ready === null ? (
        <div className='min-h-0 flex-1 overflow-y-auto'>
          {failed ? (
            <PanelMessage>{m.pr_load_failed()}</PanelMessage>
          ) : (
            <DetailSkeleton />
          )}
        </div>
      ) : (
        <>
          <TabPanel active={tab === 'summary'}>
            <SummaryTab
              threadId={threadId}
              pullRequest={pullRequest}
              detail={ready}
              actions={actions}
            />
          </TabPanel>
          {visited.has('timeline') ? (
            <TabPanel active={tab === 'timeline'}>
              <TimelineTab
                detail={ready}
                canComment={canComment}
                actions={actions}
              />
            </TabPanel>
          ) : null}
          {visited.has('code') ? (
            <TabPanel active={tab === 'code'}>
              <CodeTab threadId={threadId} version={version ?? ''} />
            </TabPanel>
          ) : null}
        </>
      )}
    </div>
  )
}

function TabPanel({
  active,
  children
}: {
  active: boolean
  children: React.ReactNode
}) {
  return (
    <div
      role='tabpanel'
      hidden={!active}
      className='min-h-0 flex-1 overflow-y-auto'
    >
      {children}
    </div>
  )
}

/** Stands in for the detail on the first open, before anything is cached. */
function DetailSkeleton() {
  return (
    <div aria-hidden='true' className='flex animate-pulse flex-col gap-3 p-3'>
      <div className='flex gap-3'>
        <div className='bg-muted h-3 w-20 rounded' />
        <div className='bg-muted h-3 w-32 rounded' />
      </div>
      <div className='flex gap-3'>
        <div className='bg-muted h-3 w-20 rounded' />
        <div className='bg-muted h-3 w-24 rounded' />
      </div>
      <div className='bg-muted mt-2 h-3 w-24 rounded' />
      <div className='bg-muted h-3 w-full rounded' />
      <div className='bg-muted h-3 w-11/12 rounded' />
      <div className='bg-muted h-3 w-4/5 rounded' />
      <div className='bg-muted h-3 w-2/3 rounded' />
    </div>
  )
}

function ChecksSummary({ checks }: { checks: PullRequestCheck[] }) {
  if (checks.length === 0) {
    return (
      <span className='text-muted-foreground ml-auto text-xs'>
        {m.pr_no_checks()}
      </span>
    )
  }
  const failing = checks.filter((check) => check.state === 'failing').length
  const pending = checks.filter((check) => check.state === 'pending').length
  const overall: ChecksState =
    failing > 0 ? 'failing' : pending > 0 ? 'pending' : 'passing'
  const { icon, color } = CHECK_STATE[overall]
  const label =
    failing > 0
      ? m.checks_failing_count({ count: failing })
      : pending > 0
        ? m.checks_pending_count({ count: pending })
        : m.checks_all_passed()

  return (
    <Popover>
      <PopoverTrigger
        render={
          <button
            type='button'
            className='hover:bg-accent focus-visible:ring-ring ml-auto flex items-center gap-1.5 rounded-md px-2 py-1 text-xs outline-none focus-visible:ring-2'
          />
        }
      >
        <HugeiconsIcon
          icon={icon}
          strokeWidth={2}
          className={cn('size-3.5', color)}
        />
        <span className={cn(overall !== 'passing' && color)}>{label}</span>
      </PopoverTrigger>
      <PopoverContent align='end' className='w-72 gap-0 p-1'>
        <ul className='flex max-h-80 flex-col overflow-y-auto'>
          {checks.map((check) => {
            const state = CHECK_STATE[check.state]
            const url = check.url
            return (
              <li key={check.name}>
                <button
                  type='button'
                  disabled={!url}
                  onClick={() => url && void openUrl(url)}
                  className='hover:bg-accent focus-visible:ring-ring flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs outline-none focus-visible:ring-2 disabled:hover:bg-transparent'
                >
                  <HugeiconsIcon
                    icon={state.icon}
                    strokeWidth={2}
                    className={cn('size-3.5 shrink-0', state.color)}
                  />
                  <span className='truncate'>{check.name}</span>
                </button>
              </li>
            )
          })}
        </ul>
      </PopoverContent>
    </Popover>
  )
}
