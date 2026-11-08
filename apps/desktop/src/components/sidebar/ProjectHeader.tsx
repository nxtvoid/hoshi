import { ProjectIcon } from './ProjectIcon'
import { m } from '@/paraglide/messages'
import { useAppStore } from '@/state/app-store'
import { HugeiconsIcon } from '@hugeicons/react'
import { Button } from '@hoshi/ui/components/button'
import { Folder01Icon, MoreHorizontalIcon } from '@hugeicons/core-free-icons'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger
} from '@hoshi/ui/components/dropdown-menu'

export function ProjectHeader() {
  const project = useAppStore((state) =>
    state.projects.find((candidate) => candidate.id === state.selectedProjectId)
  )
  const removeProject = useAppStore((state) => state.removeProject)
  if (!project) return null

  return (
    <div className='flex h-8 shrink-0 items-center gap-1 pr-1 pl-2'>
      <span className='flex min-w-0 flex-1 items-center gap-2 text-sm font-semibold'>
        <ProjectIcon
          projectId={project.id}
          className='size-3.5 rounded-[25%]'
          fallback={
            <HugeiconsIcon
              icon={Folder01Icon}
              strokeWidth={2}
              className='text-muted-foreground size-3.5 shrink-0'
            />
          }
        />
        <span className='truncate'>{project.title}</span>
      </span>
      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <Button
              variant='ghost'
              size='icon-xs'
              aria-label={m.project_actions()}
            />
          }
        >
          <HugeiconsIcon icon={MoreHorizontalIcon} strokeWidth={2} />
        </DropdownMenuTrigger>
        <DropdownMenuContent align='end' className='w-44'>
          <DropdownMenuItem
            variant='destructive'
            onClick={() => removeProject(project.id)}
          >
            {m.remove_project()}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  )
}
