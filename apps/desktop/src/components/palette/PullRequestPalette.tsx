import { useEffect, useMemo, useState } from 'react'
import { m } from '@/paraglide/messages'
import { invoke } from '@tauri-apps/api/core'
import { useAppStore } from '@/state/app-store'
import { HugeiconsIcon } from '@hugeicons/react'
import { GitBranchIcon } from '@hugeicons/core-free-icons'
import type { PullRequest } from '@/bindings/PullRequest'
import type { CommandError } from '@/bindings/CommandError'
import { useSidebarStore } from '@/state/sidebar-store'
import { PullRequestIcon } from '@/components/pull-request/PullRequestIcon'
import { forgeErrorMessage } from '@/components/panel/pull-request/use-pull-request-actions'
import { type PaletteGroup, PaletteFrame } from './PaletteFrame'

const SEARCH_DELAY_MS = 200

function startErrorMessage(error: CommandError): string {
  switch (error.code) {
    case 'worktreeFailed':
      return m.pull_request_thread_failed({ detail: error.detail })
    case 'notAGitRepository':
      return m.error_not_a_git_repository()
    case 'forgeUnavailable':
      return m.pull_requests_need_forge()
    default:
      return forgeErrorMessage(error)
  }
}

/**
 * Open pull requests of the selected project's repository; picking one opens its thread, or
 * starts one in a worktree on the pull request's branch.
 */
export function PullRequestPalette({ onClose }: { onClose: () => void }) {
  const projectId = useAppStore((state) => state.selectedProjectId)
  const threads = useAppStore((state) => state.threads)
  const pullRequests = useAppStore((state) => state.pullRequests)
  const startThread = useAppStore((state) => state.startThreadFromPullRequest)
  const setView = useSidebarStore((state) => state.setView)
  const [query, setQuery] = useState('')
  const [results, setResults] = useState<PullRequest[] | null>(null)
  const [error, setError] = useState<CommandError | null>(null)

  useEffect(() => {
    if (!projectId) return
    let current = true
    const timer = setTimeout(() => {
      invoke<PullRequest[]>('search_pull_requests', { projectId, query })
        .then((found) => {
          if (!current) return
          setResults(
            found.filter(
              (pullRequest) =>
                pullRequest.state === 'open' || pullRequest.state === 'draft'
            )
          )
          setError(null)
        })
        .catch((failure: CommandError) => {
          if (!current) return
          setResults([])
          setError(failure)
        })
    }, SEARCH_DELAY_MS)
    return () => {
      current = false
      clearTimeout(timer)
    }
  }, [projectId, query])

  const groups = useMemo((): PaletteGroup[] => {
    if (!results || results.length === 0) return []
    const threadOf = (pullRequest: PullRequest) =>
      threads.find(
        (thread) =>
          thread.projectId === projectId &&
          !thread.archived &&
          (pullRequests[thread.id]?.number === pullRequest.number ||
            thread.branch === pullRequest.headBranch)
      )
    return [
      {
        label: m.palette_pull_requests(),
        entries: results.map((pullRequest) => {
          const existing = threadOf(pullRequest)
          return {
            key: `pr:${pullRequest.number}`,
            run: async () => {
              setView('project')
              const failure = await startThread(pullRequest)
              setError(failure)
              return failure === null
            },
            content: (
              <>
                <PullRequestIcon
                  state={pullRequest.state}
                  className='size-4 shrink-0'
                />
                <span className='text-muted-foreground shrink-0 tabular-nums'>
                  #{pullRequest.number}
                </span>
                <span className='min-w-0 truncate'>{pullRequest.title}</span>
                <span className='text-muted-foreground ml-auto flex min-w-0 shrink items-center gap-1.5 text-xs'>
                  {existing ? (
                    <span className='shrink-0'>{m.open_thread()}</span>
                  ) : (
                    <>
                      <HugeiconsIcon
                        icon={GitBranchIcon}
                        strokeWidth={2}
                        className='size-3 shrink-0'
                      />
                      <span className='max-w-40 truncate font-mono text-[0.6875rem]'>
                        {pullRequest.headBranch}
                      </span>
                    </>
                  )}
                </span>
              </>
            )
          }
        })
      }
    ]
  }, [results, threads, pullRequests, projectId, startThread, setView])

  return (
    <PaletteFrame
      query={query}
      onQueryChange={setQuery}
      placeholder={m.palette_pull_request_placeholder()}
      groups={groups}
      empty={
        results === null
          ? m.loading_pull_requests()
          : error
            ? null
            : m.no_open_pull_requests()
      }
      status={
        error ? (
          <p
            role='alert'
            className='border-border text-destructive border-b px-4 py-2 text-xs text-pretty'
          >
            {startErrorMessage(error)}
          </p>
        ) : null
      }
      onClose={onClose}
    />
  )
}
