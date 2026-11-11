import { cn } from '@hoshi/utils'
import { CSS } from '@dnd-kit/utilities'
import { type RefObject, useRef, useState } from 'react'
import { WorktreeSetupDialog } from './WorktreeSetupDialog'
import { ProjectIcon } from './ProjectIcon'
import { RailInboxButton } from './InboxButton'
import { RailChatsButton } from './ChatsButton'
import { SIGNAL_BORDER } from './signal-border'
import { m } from '@/paraglide/messages'
import { useAppStore } from '@/state/app-store'
import { useSidebarStore } from '@/state/sidebar-store'
import { useUnseenStore } from '@/state/unseen-store'
import { type ProjectSignal, projectSignal } from '@/state/thread-signals'
import type { Project } from '@/bindings/Project'
import { SortableContext, useSortable } from '@dnd-kit/sortable'
import { moveProject, orderProjects } from '@/state/project-order'
import {
  restrictToParentElement,
  restrictToVerticalAxis
} from '@dnd-kit/modifiers'
import {
  DndContext,
  type DragEndEvent,
  PointerSensor,
  useSensor,
  useSensors
} from '@dnd-kit/core'
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

const SIGNAL_LABEL: Record<ProjectSignal, () => string> = {
  error: m.status_error,
  waiting: m.status_waiting_for_input,
  done: m.status_completed,
  working: m.status_working
}

const MODIFIERS = [restrictToVerticalAxis, restrictToParentElement]

export function ProjectRail() {
  const projects = useAppStore((state) => state.projects)
  const projectOrder = useSidebarStore((state) => state.projectOrder)
  const setProjectOrder = useSidebarStore((state) => state.setProjectOrder)
  const selectedProjectId = useAppStore((state) => state.selectedProjectId)
  const inProject = useSidebarStore((state) => state.view === 'project')
  const openThreadProjectId = useAppStore(
    (state) =>
      state.threads.find((thread) => thread.id === state.selectedThreadId)
        ?.projectId
  )
  const hasRail = useHasRail()
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } })
  )
  const dragged = useRef(false)
  if (!hasRail) return null

  const ordered = orderProjects(projects, projectOrder)
  const ids = ordered.map((project) => project.id)
  const onDragEnd = ({ active, over }: DragEndEvent) => {
    if (!over) return
    setProjectOrder(moveProject(ids, String(active.id), String(over.id)))
  }

  return (
    <nav
      onPointerDownCapture={() => {
        dragged.current = false
      }}
      className='border-sidebar-border flex w-12 shrink-0 flex-col items-center gap-1.5 overflow-y-auto border-r py-2'
    >
      <RailInboxButton />
      <RailChatsButton />
      {projects.length > 0 ? (
        <div className='bg-sidebar-border my-0.5 h-px w-6 shrink-0' />
      ) : null}
      <DndContext
        sensors={sensors}
        modifiers={MODIFIERS}
        onDragStart={() => {
          dragged.current = true
        }}
        onDragEnd={onDragEnd}
      >
        <SortableContext items={ids}>
          <div className='flex flex-col items-center gap-1.5'>
            {ordered.map((project) => (
              <ProjectButton
                key={project.id}
                project={project}
                selected={inProject && project.id === selectedProjectId}
                holdsOpenThread={
                  !inProject && project.id === openThreadProjectId
                }
                dragged={dragged}
              />
            ))}
          </div>
        </SortableContext>
      </DndContext>
    </nav>
  )
}

/** The rail shows once there is a project or a chat to go to. */
function useHasRail() {
  return useAppStore(
    (state) =>
      state.projects.length > 0 ||
      state.threads.some((thread) => thread.projectId === null)
  )
}

function ProjectButton({
  project,
  selected,
  holdsOpenThread,
  dragged
}: {
  project: Project
  selected: boolean
  /** In the inbox: the open thread is this project's. */
  holdsOpenThread: boolean
  /** Set once a drag starts, so the release that drops the project doesn't also open it. */
  dragged: RefObject<boolean>
}) {
  const selectProject = useAppStore((state) => state.selectProject)
  const removeProject = useAppStore((state) => state.removeProject)
  const [editingSetup, setEditingSetup] = useState(false)
  const setView = useSidebarStore((state) => state.setView)
  const unseen = useUnseenStore((state) => state.unseen)
  const signal = useAppStore((state) =>
    projectSignal(state.threads, project.id, unseen)
  )
  const { setNodeRef, listeners, transform, transition, isDragging } =
    useSortable({ id: project.id })

  return (
    <ContextMenu>
      <ContextMenuTrigger
        render={
          <div
            ref={setNodeRef}
            style={{
              transform: CSS.Translate.toString(transform),
              transition
            }}
            className={cn(isDragging && 'relative z-10')}
            {...listeners}
          />
        }
      >
        <Tooltip disabled={isDragging}>
          <TooltipTrigger
            render={
              <button
                type='button'
                aria-label={
                  signal
                    ? m.project_with_status({
                        project: project.title,
                        status: SIGNAL_LABEL[signal]()
                      })
                    : project.title
                }
                aria-current={selected || undefined}
                onClick={() => {
                  if (dragged.current) return
                  setView('project')
                  selectProject(project.id)
                }}
                className={cn(
                  'border-sidebar-border hover:bg-sidebar-accent focus-visible:ring-sidebar-ring relative flex size-8 shrink-0 items-center justify-center rounded-lg border text-[0.6875rem] font-semibold uppercase outline-none focus-visible:ring-2',
                  selected &&
                    'border-sidebar-ring bg-sidebar-accent text-sidebar-accent-foreground',
                  holdsOpenThread && 'bg-sidebar-accent/60',
                  isDragging && 'bg-sidebar-accent shadow-md',
                  signal && [
                    SIGNAL_BORDER[signal],
                    'motion-safe:animate-signal'
                  ]
                )}
              />
            }
          >
            <ProjectIcon
              projectId={project.id}
              className='size-5 rounded-[25%]'
              fallback={project.title.slice(0, 2)}
            />
          </TooltipTrigger>
          <TooltipContent
            side='right'
            sideOffset={12}
            className='flex-col items-start gap-0'
          >
            <span className='font-medium'>{project.title}</span>
            <span className='opacity-70'>{project.workspaceRoot}</span>
            {signal ? <span>{SIGNAL_LABEL[signal]()}</span> : null}
          </TooltipContent>
        </Tooltip>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuItem onClick={() => setEditingSetup(true)}>
          {m.worktree_setup_menu()}
        </ContextMenuItem>
        <ContextMenuItem
          variant='destructive'
          onClick={() => removeProject(project.id)}
        >
          {m.remove_project()}
        </ContextMenuItem>
      </ContextMenuContent>
      {editingSetup ? (
        <WorktreeSetupDialog
          project={project}
          onClose={() => setEditingSetup(false)}
        />
      ) : null}
    </ContextMenu>
  )
}
