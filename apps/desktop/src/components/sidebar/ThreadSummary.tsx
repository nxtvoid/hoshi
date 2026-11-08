import type { ReactNode } from 'react'
import { m } from '@/paraglide/messages'
import { ProjectIcon } from './ProjectIcon'
import { getLocale } from '@/paraglide/runtime'
import type { Thread } from '@/bindings/Thread'
import { useAppStore } from '@/state/app-store'
import { HugeiconsIcon } from '@hugeicons/react'
import { formatRelativeTime } from '@hoshi/utils'
import { useModelsStore } from '@/state/models-store'
import { ClaudeIcon } from '@/components/brand/ProviderIcon'
import {
  BubbleChatIcon,
  Folder01Icon,
  GitBranchIcon,
  GitForkIcon
} from '@hugeicons/core-free-icons'

const ICON = 'text-muted-foreground size-3.5 shrink-0'

/** What a thread is at a glance: its whole title, where it works, and with which model. */
export function ThreadSummary({ thread }: { thread: Thread }) {
  const project = useAppStore((state) =>
    state.projects.find((candidate) => candidate.id === thread.projectId)
  )
  const catalog = useModelsStore((state) => state.catalog)
  const slug = thread.model ?? catalog?.defaultModel ?? null
  const model = catalog?.models.find((candidate) => candidate.slug === slug)
  const modelName = model?.name ?? slug

  return (
    <div className='flex flex-col gap-2'>
      <p className='text-sm leading-snug font-medium text-pretty'>
        {thread.title || m.untitled_thread()}
      </p>
      <div className='flex flex-col gap-1.5 text-xs'>
        {thread.projectId === null ? (
          <Fact
            icon={
              <HugeiconsIcon
                icon={BubbleChatIcon}
                strokeWidth={2}
                className={ICON}
              />
            }
          >
            {m.chat()}
          </Fact>
        ) : project ? (
          <Fact
            icon={
              <ProjectIcon
                projectId={project.id}
                className='size-3.5 rounded-[25%]'
                fallback={
                  <HugeiconsIcon
                    icon={Folder01Icon}
                    strokeWidth={2}
                    className={ICON}
                  />
                }
              />
            }
          >
            {project.title}
          </Fact>
        ) : null}
        {thread.projectId === null ? null : (
          <Fact
            icon={
              <HugeiconsIcon
                icon={thread.worktreePath ? GitForkIcon : Folder01Icon}
                strokeWidth={2}
                className={ICON}
              />
            }
          >
            {thread.worktreePath
              ? thread.worktreeReleased
                ? m.summary_worktree_released()
                : m.summary_worktree()
              : m.workspace_local()}
          </Fact>
        )}
        {thread.branch ? (
          <Fact
            icon={
              <HugeiconsIcon
                icon={GitBranchIcon}
                strokeWidth={2}
                className={ICON}
              />
            }
          >
            <span className='font-mono text-[0.6875rem]'>{thread.branch}</span>
          </Fact>
        ) : null}
        {modelName ? (
          <Fact icon={<ClaudeIcon className='text-provider-claude size-3.5' />}>
            {modelName}
          </Fact>
        ) : null}
      </div>
      <p className='text-muted-foreground text-xs'>
        {m.thread_updated({
          time: formatRelativeTime(thread.updatedAt, new Date(), getLocale())
        })}
      </p>
    </div>
  )
}

function Fact({ icon, children }: { icon: ReactNode; children: ReactNode }) {
  return (
    <span className='flex min-w-0 items-center gap-2'>
      {icon}
      <span className='truncate'>{children}</span>
    </span>
  )
}
