import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { useAppStore } from '@/state/app-store'
import { HugeiconsIcon } from '@hugeicons/react'
import type { Thread } from '@/bindings/Thread'
import { usePaletteStore } from '@/state/palette-store'
import { viewOf } from '@/state/scope'
import { useSidebarStore } from '@/state/sidebar-store'
import { useNavigate } from '@tanstack/react-router'
import { useMemo, useState } from 'react'
import { useNewThread } from './use-new-thread'
import { NewThreadPalette } from './NewThreadPalette'
import { AddProjectPalette } from './AddProjectPalette'
import { PullRequestPalette } from './PullRequestPalette'
import { ClaudeIcon } from '@/components/brand/ProviderIcon'
import { ProjectIcon } from '@/components/sidebar/ProjectIcon'
import { pullRequestProblem } from '@/lib/pull-request-problem'
import { Dialog, DialogContent, DialogTitle } from '@hoshi/ui/components/dialog'
import { PullRequestIcon } from '@/components/pull-request/PullRequestIcon'
import { PALETTE_ICON, type PaletteGroup, PaletteFrame } from './PaletteFrame'
import {
  type PaletteAction,
  PALETTE_ACTIONS,
  searchPalette
} from '@/state/palette'
import {
  BarChartIcon,
  FolderAddIcon,
  Folder01Icon,
  GitBranchIcon,
  GitForkIcon,
  GitPullRequestIcon,
  InboxIcon,
  PencilEdit02Icon,
  Settings02Icon
} from '@hugeicons/core-free-icons'

const ACTION_LABELS: Record<PaletteAction, () => string> = {
  newThread: m.new_thread,
  newThreadFromPullRequest: m.new_thread_from_pull_request,
  addProject: m.add_project,
  inbox: m.inbox,
  settings: m.settings,
  usage: m.usage
}

const ACTION_ICONS = {
  newThread: PencilEdit02Icon,
  newThreadFromPullRequest: GitPullRequestIcon,
  addProject: FolderAddIcon,
  inbox: InboxIcon,
  settings: Settings02Icon,
  usage: BarChartIcon
} satisfies Record<PaletteAction, unknown>

/** Actions that work on the selected project. */
const PROJECT_ACTIONS: PaletteAction[] = [
  'newThread',
  'newThreadFromPullRequest'
]

/** Search over every thread, branch, pull request and project, plus a few app actions. */
export function PaletteDialog() {
  const open = usePaletteStore((state) => state.open)
  const mode = usePaletteStore((state) => state.mode)
  const setOpen = usePaletteStore((state) => state.setOpen)

  const close = () => setOpen(false)
  return (
    <Dialog open={open} onOpenChange={(next) => setOpen(next)}>
      <DialogContent
        showCloseButton={false}
        className='top-[12vh] translate-y-0 gap-0 overflow-hidden p-0 sm:max-w-xl'
      >
        <DialogTitle className='sr-only'>
          {mode === 'pullRequests'
            ? m.new_thread_from_pull_request()
            : mode === 'addProject'
              ? m.add_project()
              : mode === 'newThread'
                ? m.new_thread()
                : m.palette_label()}
        </DialogTitle>
        {!open ? null : mode === 'pullRequests' ? (
          <PullRequestPalette onClose={close} />
        ) : mode === 'newThread' ? (
          <NewThreadPalette
            onClose={close}
            onBack={() => setOpen(true, 'all')}
          />
        ) : mode === 'addProject' ? (
          <AddProjectPalette
            onClose={close}
            onBack={() => setOpen(true, 'all')}
          />
        ) : (
          <PaletteBody onClose={close} />
        )}
      </DialogContent>
    </Dialog>
  )
}

function PaletteBody({ onClose }: { onClose: () => void }) {
  const threads = useAppStore((state) => state.threads)
  const projects = useAppStore((state) => state.projects)
  const pullRequests = useAppStore((state) => state.pullRequests)
  const hasProject = useAppStore((state) => state.selectedProjectId !== null)
  const selectThread = useAppStore((state) => state.selectThread)
  const selectProject = useAppStore((state) => state.selectProject)
  const newThread = useNewThread()
  const setView = useSidebarStore((state) => state.setView)
  const setPaletteOpen = usePaletteStore((state) => state.setOpen)
  const navigate = useNavigate()
  const [query, setQuery] = useState('')

  const groups = useMemo(() => {
    const results = searchPalette(query, {
      threads,
      projects,
      pullRequests,
      actionLabels: Object.fromEntries(
        PALETTE_ACTIONS.map((action) => [action, ACTION_LABELS[action]()])
      ) as Record<PaletteAction, string>
    })
    const projectTitle = new Map(
      projects.map((project) => [project.id, project.title])
    )
    const runAction = (action: PaletteAction) => {
      switch (action) {
        case 'newThread':
          return newThread() ? Promise.resolve(false) : undefined
        case 'newThreadFromPullRequest':
          setPaletteOpen(true, 'pullRequests')
          return Promise.resolve(false)
        case 'addProject':
          setPaletteOpen(true, 'addProject')
          return Promise.resolve(false)
        case 'inbox':
          return setView('inbox')
        case 'settings':
          return void navigate({
            to: '/settings/$section',
            params: { section: 'general' }
          })
        case 'usage':
          return void navigate({ to: '/usage' })
      }
    }
    const built: PaletteGroup[] = [
      {
        label: query.trim() ? m.palette_threads() : m.palette_recent(),
        entries: results.threads.map((thread) => ({
          key: `thread:${thread.id}`,
          run: () => {
            setView(viewOf(thread))
            selectThread(thread.id)
          },
          content: (
            <ThreadEntry
              thread={thread}
              project={
                thread.projectId === null
                  ? m.chat()
                  : projects.length > 1
                    ? projectTitle.get(thread.projectId)
                    : undefined
              }
            />
          )
        }))
      },
      {
        label: m.palette_projects(),
        entries: results.projects.map((project) => ({
          key: `project:${project.id}`,
          run: () => {
            setView('project')
            selectProject(project.id)
          },
          content: (
            <>
              <ProjectIcon
                projectId={project.id}
                className='size-4 rounded-[25%]'
                fallback={
                  <HugeiconsIcon
                    icon={Folder01Icon}
                    strokeWidth={2}
                    className={PALETTE_ICON}
                  />
                }
              />
              <span className='truncate'>{project.title}</span>
              <span className='text-muted-foreground ml-auto min-w-0 truncate text-xs'>
                {project.workspaceRoot}
              </span>
            </>
          )
        }))
      },
      {
        label: m.palette_actions(),
        entries: results.actions
          .filter((action) => hasProject || !PROJECT_ACTIONS.includes(action))
          .map((action) => ({
            key: `action:${action}`,
            run: () => runAction(action),
            content: (
              <>
                <HugeiconsIcon
                  icon={ACTION_ICONS[action]}
                  strokeWidth={2}
                  className={PALETTE_ICON}
                />
                <span className='truncate'>{ACTION_LABELS[action]()}</span>
              </>
            )
          }))
      }
    ]
    return built.filter((group) => group.entries.length > 0)
  }, [
    query,
    threads,
    projects,
    pullRequests,
    hasProject,
    selectThread,
    selectProject,
    newThread,
    setView,
    setPaletteOpen,
    navigate
  ])

  return (
    <PaletteFrame
      query={query}
      onQueryChange={setQuery}
      placeholder={m.palette_placeholder()}
      groups={groups}
      empty={m.palette_no_results()}
      onClose={onClose}
    />
  )
}

function ThreadEntry({
  thread,
  project
}: {
  thread: Thread
  project: string | undefined
}) {
  const pullRequest = useAppStore((state) => state.pullRequests[thread.id])

  return (
    <>
      <ClaudeIcon className='text-provider-claude size-3.5 shrink-0' />
      <span
        className={cn(
          'min-w-0 truncate',
          thread.archived && 'text-muted-foreground'
        )}
      >
        {thread.title || m.untitled_thread()}
      </span>
      <span className='text-muted-foreground ml-auto flex min-w-0 shrink items-center gap-1.5 text-xs'>
        {project ? <span className='shrink-0'>{project}</span> : null}
        {thread.branch ? (
          <>
            <HugeiconsIcon
              icon={thread.worktreePath ? GitForkIcon : GitBranchIcon}
              strokeWidth={2}
              className='size-3 shrink-0'
            />
            <span className='max-w-40 truncate font-mono text-[0.6875rem]'>
              {thread.branch}
            </span>
          </>
        ) : null}
        {pullRequest ? (
          <span className='flex shrink-0 items-center gap-1'>
            <PullRequestIcon
              state={pullRequest.state}
              problem={pullRequestProblem(pullRequest)}
              className='size-3'
            />
            <span className='tabular-nums'>#{pullRequest.number}</span>
          </span>
        ) : null}
      </span>
    </>
  )
}
