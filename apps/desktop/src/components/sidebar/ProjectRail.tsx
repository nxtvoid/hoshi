import { cn } from '@/lib/utils'
import { useAppStore } from '@/state/app-store'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@/components/ui/tooltip'

export function ProjectRail() {
  const projects = useAppStore((state) => state.projects)
  const selectedProjectId = useAppStore((state) => state.selectedProjectId)
  const selectProject = useAppStore((state) => state.selectProject)

  return (
    <nav className='border-sidebar-border flex w-14 shrink-0 flex-col items-center gap-2 overflow-y-auto border-r py-2'>
      {projects.map((project) => {
        const selected = project.id === selectedProjectId
        return (
          <Tooltip key={project.id}>
            <TooltipTrigger
              render={
                <button
                  type='button'
                  aria-label={project.name}
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
              {project.name.slice(0, 2)}
            </TooltipTrigger>
            <TooltipContent side='right'>{project.name}</TooltipContent>
          </Tooltip>
        )
      })}
    </nav>
  )
}
