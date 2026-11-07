import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { useAppStore } from '@/state/app-store'
import type { Project } from '@/bindings/Project'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@hoshi/ui/components/tooltip'
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger
} from '@hoshi/ui/components/context-menu'

export function ProjectRail() {
  const projects = useAppStore((state) => state.projects)
  const selectedProjectId = useAppStore((state) => state.selectedProjectId)

  return (
    <nav className='border-sidebar-border flex w-14 shrink-0 flex-col items-center gap-2 overflow-y-auto border-r py-2'>
      {projects.map((project) => (
        <ProjectButton
          key={project.id}
          project={project}
          selected={project.id === selectedProjectId}
        />
      ))}
    </nav>
  )
}

function ProjectButton({
  project,
  selected
}: {
  project: Project
  selected: boolean
}) {
  const selectProject = useAppStore((state) => state.selectProject)
  const removeProject = useAppStore((state) => state.removeProject)

  return (
    <ContextMenu>
      <ContextMenuTrigger render={<div />}>
        <Tooltip>
          <TooltipTrigger
            render={
              <button
                type='button'
                aria-label={project.title}
                aria-current={selected || undefined}
                onClick={() => selectProject(project.id)}
                className={cn(
                  'border-sidebar-border hover:bg-sidebar-accent focus-visible:ring-sidebar-ring flex size-10 shrink-0 items-center justify-center rounded-lg border text-xs font-semibold uppercase outline-none focus-visible:ring-2',
                  selected &&
                    'border-sidebar-foreground bg-sidebar-accent text-sidebar-accent-foreground'
                )}
              />
            }
          >
            {project.title.slice(0, 2)}
          </TooltipTrigger>
          <TooltipContent
            side='right'
            sideOffset={15}
            className='flex-col items-start gap-0'
          >
            <span className='font-medium'>{project.title}</span>
            <span className='opacity-70'>{project.workspaceRoot}</span>
          </TooltipContent>
        </Tooltip>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuItem
          variant='destructive'
          onClick={() => removeProject(project.id)}
        >
          {m.remove_project()}
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  )
}
