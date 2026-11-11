import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { ThreadRow } from './ThreadRow'
import { ThreadCardHost } from './ThreadCard'
import { CleanUpMergedButton } from './CleanUpMergedButton'
import { ProjectHeader } from './ProjectHeader'
import { useAppStore } from '@/state/app-store'
import type { Project } from '@/bindings/Project'
import { useInChat, useScopeProjectId } from '@/state/use-scope'
import { usePaletteStore } from '@/state/palette-store'
import { HugeiconsIcon } from '@hugeicons/react'
import { Button } from '@hoshi/ui/components/button'
import { useSidebarStore } from '@/state/sidebar-store'
import { draftKey, hasDraft, useDraftStore } from '@/state/draft-store'
import { useSidebarSections } from './use-sidebar-sections'
import type {
  SectionId,
  SidebarView,
  ThreadSection
} from '@/state/thread-sections'
import {
  ArrowLeft01Icon,
  ArrowRight01Icon,
  BubbleChatIcon,
  Cancel01Icon,
  InboxIcon,
  PencilEdit02Icon
} from '@hugeicons/core-free-icons'
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger
} from '@hoshi/ui/components/context-menu'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@hoshi/ui/components/tooltip'

const SECTION_LABELS: Record<SectionId, () => string> = {
  pinned: m.section_pinned,
  attention: m.section_attention,
  active: m.section_active,
  inactive: m.section_inactive,
  review: m.section_review,
  reviewing: m.section_reviewing,
  done: m.section_done,
  completed: m.status_completed
}

export function ThreadList() {
  const ready = useAppStore((state) => state.ready)
  const search = useAppStore((state) => state.search)
  const hasProjects = useAppStore((state) => state.projects.length > 0)
  const projectId = useAppStore((state) => state.selectedProjectId)
  const openPalette = usePaletteStore((state) => state.setOpen)
  const view = useSidebarStore((state) => state.view)
  const filter = useSidebarStore((state) => state.filter)
  const sections = useSidebarSections()
  const inbox = view === 'inbox'
  const chats = view === 'chats'

  if (!ready) return <div className='flex-1' />
  if (!hasProjects && !chats) {
    return (
      <div className='flex flex-1 flex-col items-center gap-3 p-4 text-center'>
        <p className='text-muted-foreground text-xs'>{m.no_projects()}</p>
        <Button
          variant='outline'
          size='sm'
          onClick={() => openPalette(true, 'addProject')}
        >
          {m.add_project()}
        </Button>
      </div>
    )
  }
  if (!projectId && view === 'project') {
    return <EmptyMessage>{m.no_project_selected()}</EmptyMessage>
  }

  const onlyActive =
    !inbox && sections.length === 1 && sections[0]?.id === 'active'
  const narrowed = search !== '' || filter !== 'all'

  return (
    <div className='flex min-w-0 flex-1 flex-col pt-1.5'>
      <ThreadCardHost />
      {inbox ? (
        <ViewHeader icon={InboxIcon} title={m.inbox()} />
      ) : chats ? (
        <ViewHeader icon={BubbleChatIcon} title={m.chats()} />
      ) : (
        <ProjectHeader />
      )}
      <NewThreadDrafts view={view} />
      {sections.length === 0 ? (
        <EmptyMessage>
          {narrowed
            ? m.no_matching_threads()
            : inbox
              ? m.inbox_empty()
              : chats
                ? m.no_chats()
                : m.no_threads()}
        </EmptyMessage>
      ) : (
        <div className='flex min-h-0 flex-1 flex-col gap-1 overflow-y-auto px-1.5 pb-2'>
          {sections.map((section) => (
            <Section
              key={section.id}
              section={section}
              showHeader={!onlyActive}
              forceOpen={search !== ''}
              showProject={inbox}
            />
          ))}
        </div>
      )}
    </div>
  )
}

/**
 * New threads with unsent text: this project's or the chats' one, or every one in the inbox.
 * The draft on screen is left out.
 */
function NewThreadDrafts({ view }: { view: SidebarView }) {
  const drafts = useDraftStore((state) => state.drafts)
  const draftImages = useDraftStore((state) => state.images)
  const forgetDrafts = useDraftStore((state) => state.forget)
  const projects = useAppStore((state) => state.projects)
  const selectedProjectId = useAppStore((state) => state.selectedProjectId)
  const threadOpen = useAppStore((state) => state.selectedThreadId !== null)
  const scopeProjectId = useScopeProjectId()
  const chat = useInChat()
  const selectProject = useAppStore((state) => state.selectProject)
  const openDraft = useAppStore((state) => state.openDraft)
  const setView = useSidebarStore((state) => state.setView)
  const shown = threadOpen ? null : draftKey(null, scopeProjectId, chat)

  const wanted =
    view === 'inbox'
      ? [
          ...new Set([...Object.keys(drafts), ...Object.keys(draftImages)])
        ].filter((key) => key.startsWith('new:'))
      : [draftKey(null, selectedProjectId, view === 'chats')]
  const rows = wanted.flatMap((key): DraftRow[] => {
    if (!hasDraft({ drafts, images: draftImages }, key) || key === shown) {
      return []
    }
    if (key === 'new:chat') return [{ key, project: null }]
    const project = projects.find(({ id }) => `new:${id}` === key)
    return project ? [{ key, project }] : []
  })
  if (rows.length === 0) return null

  return (
    <div className='flex flex-col gap-px px-1.5'>
      {rows.map(({ key, project }) => (
        <ContextMenu key={key}>
          <ContextMenuTrigger
            render={
              <div className='group/draft bg-draft-surface hover:bg-draft/10 relative flex items-center rounded-md' />
            }
          >
            <button
              type='button'
              onClick={() => {
                setView(project ? 'project' : 'chats')
                if (project) selectProject(project.id)
                else openDraft()
              }}
              className='focus-visible:ring-sidebar-ring flex min-w-0 flex-1 items-center gap-2 rounded-md px-2 py-1.5 text-left outline-none focus-visible:ring-2'
            >
              <HugeiconsIcon
                icon={PencilEdit02Icon}
                strokeWidth={2}
                className='text-draft size-3.5 shrink-0'
              />
              <span className='min-w-0 flex-1 truncate text-sm'>
                {!project
                  ? m.new_chat()
                  : view === 'inbox'
                    ? project.title
                    : m.new_thread()}
              </span>
              <span className='text-draft shrink-0 text-xs italic group-focus-within/draft:invisible group-hover/draft:invisible'>
                {m.draft()}
              </span>
            </button>
            <Tooltip>
              <TooltipTrigger
                render={
                  <Button
                    variant='ghost'
                    size='icon-xs'
                    aria-label={m.discard_draft()}
                    onClick={() => forgetDrafts([key])}
                    className='absolute right-1 opacity-0 group-focus-within/draft:opacity-100 group-hover/draft:opacity-100'
                  />
                }
              >
                <HugeiconsIcon icon={Cancel01Icon} strokeWidth={2} />
              </TooltipTrigger>
              <TooltipContent side='right' sideOffset={15}>
                {m.discard_draft()}
              </TooltipContent>
            </Tooltip>
          </ContextMenuTrigger>
          <ContextMenuContent>
            <ContextMenuItem
              variant='destructive'
              onClick={() => forgetDrafts([key])}
            >
              {m.discard_draft()}
            </ContextMenuItem>
          </ContextMenuContent>
        </ContextMenu>
      ))}
      <div className='bg-sidebar-border mx-2 my-1.5 h-px' />
    </div>
  )
}

type DraftRow = { key: string; project: Project | null }

function ViewHeader({
  icon,
  title
}: {
  icon: typeof InboxIcon
  title: string
}) {
  const hasProjects = useAppStore((state) => state.projects.length > 0)

  return (
    <div className='flex h-8 shrink-0 items-center gap-1.5 pr-1 pl-2 text-sm font-semibold'>
      {hasProjects ? (
        <BackToProject />
      ) : (
        <HugeiconsIcon
          icon={icon}
          strokeWidth={2}
          className='text-muted-foreground size-3.5 shrink-0'
        />
      )}
      <span className='truncate'>{title}</span>
    </div>
  )
}

/** Leads back to the project's threads: the lone project by name, or the one the rail picks. */
function BackToProject() {
  const lone = useAppStore((state) =>
    state.projects.length === 1 ? state.projects[0] : undefined
  )
  const selectedProjectId = useAppStore((state) => state.selectedProjectId)
  const selectProject = useAppStore((state) => state.selectProject)
  const setView = useSidebarStore((state) => state.setView)

  return (
    <>
      <button
        type='button'
        onClick={() => {
          setView('project')
          if (lone && selectedProjectId !== lone.id) selectProject(lone.id)
        }}
        className='text-muted-foreground hover:text-foreground focus-visible:ring-sidebar-ring flex max-w-1/2 min-w-0 shrink items-center gap-0.5 rounded-sm font-normal outline-none focus-visible:ring-2'
      >
        <HugeiconsIcon
          icon={ArrowLeft01Icon}
          strokeWidth={2}
          className='size-3.5 shrink-0'
        />
        <span className='truncate'>{lone ? lone.title : m.projects()}</span>
      </button>
      <span aria-hidden className='text-muted-foreground/60 font-normal'>
        /
      </span>
    </>
  )
}

function Section({
  section,
  showHeader,
  forceOpen,
  showProject
}: {
  section: ThreadSection
  showHeader: boolean
  forceOpen: boolean
  showProject: boolean
}) {
  const collapsed = useSidebarStore(
    (state) => state.collapsed[section.id] === true
  )
  const toggleSection = useSidebarStore((state) => state.toggleSection)
  const selectedThreadId = useAppStore((state) => state.selectedThreadId)
  const selectThread = useAppStore((state) => state.selectThread)
  const open = forceOpen || !collapsed
  const shown = open
    ? section.threads
    : section.threads.filter((thread) => thread.id === selectedThreadId)

  return (
    <section>
      {showHeader ? (
        <div className='flex items-center gap-1'>
          <button
            type='button'
            aria-expanded={open}
            disabled={forceOpen}
            onClick={() => toggleSection(section.id)}
            className='text-muted-foreground hover:text-sidebar-foreground focus-visible:ring-sidebar-ring flex min-w-0 flex-1 items-center gap-1 rounded-md px-2 pt-2 pb-1 text-[0.6875rem] font-semibold tracking-wide uppercase outline-none focus-visible:ring-2'
          >
            <HugeiconsIcon
              icon={ArrowRight01Icon}
              strokeWidth={2.5}
              className={cn('size-3 transition-transform', open && 'rotate-90')}
            />
            <span>{SECTION_LABELS[section.id]()}</span>
            <span className='ml-auto font-medium tabular-nums'>
              {section.threads.length}
            </span>
          </button>
          {section.id === 'done' ? (
            <CleanUpMergedButton threads={section.threads} />
          ) : null}
        </div>
      ) : null}
      <ul className='flex flex-col gap-px'>
        {shown.map((thread) => (
          <li key={thread.id}>
            <ThreadRow
              thread={thread}
              selected={thread.id === selectedThreadId}
              onSelect={selectThread}
              showProject={showProject}
            />
          </li>
        ))}
      </ul>
    </section>
  )
}

function EmptyMessage({ children }: { children: string }) {
  return (
    <p className='text-muted-foreground flex-1 p-4 text-center text-xs'>
      {children}
    </p>
  )
}
