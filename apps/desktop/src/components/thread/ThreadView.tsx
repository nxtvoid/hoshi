import type * as React from 'react'
import { useEffect, useRef, useState } from 'react'
import { cn } from '@hoshi/utils'
import { Composer } from './Composer'
import { CHAT_LANE } from '@/lib/chat-lane'
import {
  DETAILS_CARD_CLEARANCE,
  DETAILS_CARD_GAP,
  DETAILS_CARD_WIDTH,
  ThreadDetailsCard,
  ThreadDetailsToggle,
  useOpenInShortcut
} from './ThreadDetails'
import { m } from '@/paraglide/messages'
import { Transcript } from './Transcript'
import { DelegatedBanner } from './DelegationRows'
import { ThreadGitChip } from './ThreadGitChip'
import { useAppStore } from '@/state/app-store'
import { useSidebarStore } from '@/state/sidebar-store'
import { useInChat, useScopeProjectId } from '@/state/use-scope'
import { draftKey } from '@/state/draft-store'
import { ProjectIcon } from '@/components/sidebar/ProjectIcon'
import { MergedCleanupBanner } from './MergedCleanupBanner'
import { ReviewCommentsBar } from './ReviewCommentsBar'
import { HugeiconsIcon } from '@hugeicons/react'
import { LogoMark } from '@/components/brand/Logo'
import { usePanelStore } from '@/state/panel-store'
import { useTerminalStore } from '@/state/terminal-store'
import { RightPanel } from '@/components/panel/RightPanel'
import { useTranscriptStore } from '@/state/transcript-store'
import { IconButton } from '@hoshi/ui/components/icon-button'
import { WindowControls } from '@/components/window/WindowControls'
import { TerminalDrawer } from '@/components/terminal/TerminalDrawer'
import {
  BubbleChatIcon,
  LayoutBottomIcon,
  SidebarLeftIcon,
  SidebarRightIcon
} from '@hugeicons/core-free-icons'

export function ThreadView() {
  const sidebarOpen = useAppStore((state) => state.sidebarOpen)
  const toggleSidebar = useAppStore((state) => state.toggleSidebar)
  const thread = useAppStore((state) =>
    state.threads.find((candidate) => candidate.id === state.selectedThreadId)
  )
  const fromInbox = useSidebarStore((state) => state.view === 'inbox')
  const chat = useInChat()
  const scopeProjectId = useScopeProjectId()
  const projectTitle = useAppStore(
    (state) =>
      state.projects.find((project) => project.id === scopeProjectId)?.title
  )
  const workspaceRoot = useAppStore(
    (state) =>
      state.projects.find((project) => project.id === thread?.projectId)
        ?.workspaceRoot
  )
  const panelOpen = usePanelStore((state) => state.open)
  const togglePanel = usePanelStore((state) => state.toggle)
  const panelMaximized = usePanelStore((state) => state.maximized)
  const detailsOpen = usePanelStore((state) => state.detailsOpen)
  const terminalOpen = useTerminalStore((state) =>
    thread ? (state.drawers[thread.id]?.open ?? false) : false
  )
  const toggleTerminal = useTerminalStore((state) => state.toggleDrawer)
  const threadId = thread?.id
  useOpenInShortcut(thread?.id)
  const chatRef = useRef<HTMLDivElement>(null)
  const chatWidth = useElementWidth(chatRef)
  const detailsInline = chatWidth >= DETAILS_INLINE_MIN_WIDTH
  const showDetailsCard = Boolean(thread && detailsOpen && detailsInline)
  const loading = useTranscriptStore((state) => state.loading)
  const empty = useTranscriptStore((state) => state.items.length === 0)
  const working =
    thread?.status === 'working' || thread?.status === 'waitingForInput'
  const draft = !loading && empty && !working
  const composerDraft = draftKey(thread?.id ?? null, scopeProjectId, chat)

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.ctrlKey && event.altKey && event.code === 'KeyB') {
        event.preventDefault()
        togglePanel()
      } else if (
        threadId &&
        event.ctrlKey &&
        !event.altKey &&
        event.code === 'KeyJ'
      ) {
        event.preventDefault()
        toggleTerminal(threadId)
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [togglePanel, toggleTerminal, threadId])

  return (
    <main className='flex min-w-0 flex-1 flex-col'>
      <header
        data-tauri-drag-region
        className='border-border flex h-10 shrink-0 items-center gap-2 border-b pl-3'
      >
        {sidebarOpen ? null : (
          <IconButton label={m.toggle_sidebar()} onClick={toggleSidebar}>
            <HugeiconsIcon icon={SidebarLeftIcon} strokeWidth={2} />
          </IconButton>
        )}
        {thread ? (
          <>
            {fromInbox ? (
              thread.projectId ? (
                <ProjectCrumb projectId={thread.projectId} />
              ) : (
                <ChatsCrumb />
              )
            ) : null}
            <h1
              data-tauri-drag-region
              className='max-w-[40%] min-w-0 truncate text-sm font-medium'
            >
              {thread.title || m.untitled_thread()}
            </h1>
            <ThreadGitChip threadId={thread.id} />
          </>
        ) : null}
        <div className='ml-auto flex h-full shrink-0 items-center gap-1'>
          {thread ? (
            <ThreadDetailsToggle threadId={thread.id} inline={detailsInline} />
          ) : null}
          {thread ? (
            <IconButton
              label={m.toggle_terminal()}
              active={terminalOpen}
              onClick={() => toggleTerminal(thread.id)}
            >
              <HugeiconsIcon icon={LayoutBottomIcon} strokeWidth={2} />
            </IconButton>
          ) : null}
          {thread ? (
            <IconButton
              label={m.toggle_panel()}
              active={panelOpen}
              onClick={togglePanel}
            >
              <HugeiconsIcon icon={SidebarRightIcon} strokeWidth={2} />
            </IconButton>
          ) : null}
          <WindowControls />
        </div>
      </header>
      {thread ? <MergedCleanupBanner threadId={thread.id} /> : null}
      <div className='flex min-h-0 flex-1'>
        <div
          ref={chatRef}
          hidden={Boolean(thread && panelOpen && panelMaximized)}
          style={
            showDetailsCard
              ? ({
                  '--details-room': `${DETAILS_CARD_ROOM}px`,
                  '--details-gutter': `${DETAILS_CARD_CLEARANCE}px`
                } as React.CSSProperties)
              : undefined
          }
          className='relative flex min-w-0 flex-1 flex-col'
        >
          {showDetailsCard && thread ? (
            <ThreadDetailsCard threadId={thread.id} />
          ) : null}
          {!chat && projectTitle === undefined ? (
            <div className='text-muted-foreground flex flex-1 flex-col items-center justify-center gap-4 text-sm'>
              <LogoMark className='text-muted-foreground/25 size-12' />
              <p>{m.no_project_selected()}</p>
            </div>
          ) : loading ? (
            <div className='flex-1' />
          ) : draft ? (
            <div className='flex flex-1 flex-col items-center justify-center gap-8 px-4 pb-[12vh]'>
              <h2 className='text-foreground max-w-3xl text-center text-3xl font-normal tracking-tight text-balance'>
                {projectTitle === undefined
                  ? m.new_chat_prompt()
                  : m.new_thread_prompt({ project: projectTitle })}
              </h2>
              <div className='w-full'>
                <Composer
                  key={composerDraft}
                  draftKey={composerDraft}
                  working={working}
                  variant='hero'
                />
              </div>
            </div>
          ) : (
            <>
              {thread?.parentThreadId ? (
                <div className={cn(CHAT_LANE, 'w-full px-4 pt-3')}>
                  <DelegatedBanner parentThreadId={thread.parentThreadId} />
                </div>
              ) : null}
              <Transcript
                working={working}
                cwd={thread?.worktreePath ?? workspaceRoot ?? null}
              />
              <div className='relative px-4 pb-3'>
                <div
                  aria-hidden
                  className='pointer-events-none absolute inset-x-4 bottom-full'
                >
                  <div
                    className={cn(
                      CHAT_LANE,
                      'bg-background h-8 [mask-image:linear-gradient(to_top,black,transparent)] backdrop-blur-sm'
                    )}
                  />
                </div>
                {thread ? <ReviewCommentsBar threadId={thread.id} /> : null}
                <Composer
                  key={composerDraft}
                  draftKey={composerDraft}
                  working={working}
                  variant='docked'
                  cwd={thread?.worktreePath ?? workspaceRoot ?? null}
                />
              </div>
            </>
          )}
          {thread ? <TerminalDrawer threadId={thread.id} /> : null}
        </div>
        {thread && panelOpen ? <RightPanel threadId={thread.id} /> : null}
      </div>
    </main>
  )
}

/** The room the floating details card takes from the chat. */
const DETAILS_CARD_ROOM =
  DETAILS_CARD_WIDTH + DETAILS_CARD_GAP + DETAILS_CARD_CLEARANCE
/** Below this chat width the card would crowd the chat, so it becomes a popover. */
const DETAILS_INLINE_MIN_WIDTH =
  DETAILS_CARD_ROOM + DETAILS_CARD_CLEARANCE + 560

function useElementWidth(ref: React.RefObject<HTMLElement | null>) {
  const [width, setWidth] = useState(0)
  useEffect(() => {
    const element = ref.current
    if (!element) return
    const observer = new ResizeObserver(([entry]) => {
      if (entry) setWidth(entry.contentRect.width)
    })
    observer.observe(element)
    return () => observer.disconnect()
  }, [ref])
  return width
}

/** "Chats" before a chat's title in the inbox; opens the list of chats. */
function ChatsCrumb() {
  const setView = useSidebarStore((state) => state.setView)
  return (
    <>
      <button
        type='button'
        onClick={() => setView('chats')}
        className='text-muted-foreground hover:text-foreground focus-visible:ring-ring flex shrink-0 cursor-pointer items-center gap-1.5 rounded-md text-sm outline-none focus-visible:ring-2'
      >
        <HugeiconsIcon
          icon={BubbleChatIcon}
          strokeWidth={2}
          className='size-4'
        />
        <span>{m.chats()}</span>
      </button>
      <span aria-hidden className='text-muted-foreground/60 text-sm'>
        /
      </span>
    </>
  )
}

/**
 * The thread's project before its title, while the sidebar shows the inbox, where threads of
 * every project mix. Opens that project's list.
 */
function ProjectCrumb({ projectId }: { projectId: string }) {
  const project = useAppStore((state) =>
    state.projects.find((candidate) => candidate.id === projectId)
  )
  const setView = useSidebarStore((state) => state.setView)
  if (!project) return null

  return (
    <>
      <button
        type='button'
        title={project.workspaceRoot}
        onClick={() => setView('project')}
        className='text-muted-foreground hover:text-foreground focus-visible:ring-ring flex max-w-[30%] min-w-0 shrink-0 cursor-pointer items-center gap-1.5 rounded-md text-sm outline-none focus-visible:ring-2'
      >
        <ProjectIcon
          projectId={project.id}
          className='size-4 rounded-[25%]'
          fallback={
            <span className='bg-muted text-muted-foreground flex size-4 shrink-0 items-center justify-center rounded-[25%] text-[0.5rem] font-semibold uppercase'>
              {project.title.slice(0, 2)}
            </span>
          }
        />
        <span className='truncate'>{project.title}</span>
      </button>
      <span aria-hidden className='text-muted-foreground/60 text-sm'>
        /
      </span>
    </>
  )
}
