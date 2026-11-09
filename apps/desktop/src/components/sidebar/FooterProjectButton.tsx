import { ProjectIcon } from './ProjectIcon'
import { useAppStore } from '@/state/app-store'
import { useSidebarStore } from '@/state/sidebar-store'
import { IconButton } from '@hoshi/ui/components/icon-button'

/** The lone project in the sidebar footer, while there is no rail to hold it. */
export function FooterProjectButton() {
  const project = useAppStore((state) =>
    state.projects.length === 1 ? state.projects[0] : undefined
  )
  const selected = useAppStore(
    (state) => state.selectedProjectId === project?.id
  )
  const selectProject = useAppStore((state) => state.selectProject)
  const open = useSidebarStore((state) => state.view === 'project')
  const setView = useSidebarStore((state) => state.setView)
  if (!project) return null

  return (
    <IconButton
      label={project.title}
      side='top'
      active={open}
      onClick={() => {
        setView('project')
        if (!selected) selectProject(project.id)
      }}
    >
      <ProjectIcon
        projectId={project.id}
        className='size-4 rounded-[25%]'
        fallback={
          <span className='text-[0.5625rem] leading-none font-semibold uppercase'>
            {project.title.slice(0, 2)}
          </span>
        }
      />
    </IconButton>
  )
}
