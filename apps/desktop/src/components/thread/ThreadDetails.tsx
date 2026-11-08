import { m } from '@/paraglide/messages'
import { invoke } from '@tauri-apps/api/core'
import { useAppStore } from '@/state/app-store'
import { HugeiconsIcon } from '@hugeicons/react'
import type { Editor } from '@/bindings/Editor'
import { usePanelStore } from '@/state/panel-store'
import { useEditorStore } from '@/state/editor-store'
import { useDetailsStore } from '@/state/details-store'
import { EditorIcon, isMac, isWindows } from '@/components/brand/EditorIcon'
import { type ReactNode, useEffect, useRef, useState } from 'react'
import { revealItemInDir } from '@tauri-apps/plugin-opener'
import type { CommandError } from '@/bindings/CommandError'
import { Button } from '@hoshi/ui/components/button'
import { IconButton } from '@hoshi/ui/components/icon-button'
import { PullRequestIcon } from '@/components/pull-request/PullRequestIcon'
import {
  ArrowDown01Icon,
  FileDiffIcon,
  Folder01Icon,
  FolderOpenIcon,
  GitBranchIcon,
  GitForkIcon,
  LeftToRightListBulletIcon
} from '@hugeicons/core-free-icons'
import {
  Popover,
  PopoverContent,
  PopoverTrigger
} from '@hoshi/ui/components/popover'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuShortcut,
  DropdownMenuTrigger
} from '@hoshi/ui/components/dropdown-menu'

/** Width of the floating card, and the room the chat leaves it. */
export const DETAILS_CARD_WIDTH = 280
export const DETAILS_CARD_GAP = 12
export const DETAILS_CARD_CLEARANCE = 32

const EDITOR_LABEL: Record<Editor, () => string> = {
  vscode: m.editor_vscode,
  cursor: m.editor_cursor,
  windsurf: m.editor_windsurf,
  zed: m.editor_zed,
  fileManager: () =>
    isMac() ? m.finder() : isWindows() ? m.file_explorer() : m.files_app()
}

/** The open-in shortcut: Ctrl+O, or ⌘O on a Mac. */
const OPEN_SHORTCUT = () => (isMac() ? '⌘O' : 'Ctrl+O')

/** The target "Open in" uses: the one the user picked, else the first found. */
function openTarget(editors: Editor[] | null, preferred: Editor | null) {
  const found = editors ?? []
  return preferred && found.includes(preferred) ? preferred : found[0]
}

/** Ctrl+O (⌘O on a Mac) opens the thread's folder where "Open in" would. */
export function useOpenInShortcut(threadId: string | undefined) {
  const loadEditors = useDetailsStore((state) => state.loadEditors)

  useEffect(() => {
    void loadEditors()
  }, [loadEditors])

  useEffect(() => {
    if (!threadId) return
    const onKeyDown = (event: KeyboardEvent) => {
      const modifier = isMac() ? event.metaKey : event.ctrlKey
      if (
        !modifier ||
        event.shiftKey ||
        event.altKey ||
        event.code !== 'KeyO'
      ) {
        return
      }
      const { editors } = useDetailsStore.getState()
      const target = openTarget(editors, useEditorStore.getState().preferred)
      if (!target) return
      event.preventDefault()
      void invoke('open_in_editor', { threadId, editor: target }).catch(
        (error) => console.error('hoshi: failed to open the folder', error)
      )
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [threadId])
}

const ROW =
  'flex h-8 w-full min-w-0 cursor-pointer items-center gap-2.5 rounded-lg px-2.5 text-left text-[13px] font-medium text-foreground/80 outline-none transition-colors hover:bg-foreground/5 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset disabled:cursor-default disabled:opacity-60 disabled:hover:bg-transparent'
const ICON = 'size-4 shrink-0 text-muted-foreground'

function errorMessage(error: CommandError) {
  switch (error.code) {
    case 'gitFailed':
      return error.detail
    case 'threadBusy':
      return m.error_thread_busy()
    default:
      return m.error_generic()
  }
}

/**
 * The header button. With room, it shows or hides the floating card; without it, the
 * card opens as a popover from here.
 */
export function ThreadDetailsToggle({
  threadId,
  inline
}: {
  threadId: string
  inline: boolean
}) {
  return inline ? <DetailsCardToggle /> : <DetailsPopover threadId={threadId} />
}

const DETAILS_ICON = (
  <HugeiconsIcon icon={LeftToRightListBulletIcon} strokeWidth={2} />
)

function DetailsCardToggle() {
  const open = usePanelStore((state) => state.detailsOpen)
  const setOpen = usePanelStore((state) => state.setDetailsOpen)
  return (
    <IconButton
      label={m.thread_details()}
      active={open}
      onClick={() => setOpen(!open)}
    >
      {DETAILS_ICON}
    </IconButton>
  )
}

function DetailsPopover({ threadId }: { threadId: string }) {
  const [open, setOpen] = useState(false)
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        render={
          <Button
            variant={open ? 'secondary' : 'ghost'}
            size='icon-sm'
            aria-label={m.thread_details()}
            title={m.thread_details()}
          />
        }
      >
        {DETAILS_ICON}
      </PopoverTrigger>
      <PopoverContent align='end' className='w-70 gap-0 p-1.5'>
        <DetailsContent threadId={threadId} onNavigate={() => setOpen(false)} />
      </PopoverContent>
    </Popover>
  )
}

/** The details floating over the top-right corner of the chat, open until toggled. */
export function ThreadDetailsCard({ threadId }: { threadId: string }) {
  return (
    <aside
      aria-label={m.thread_details()}
      style={{
        width: DETAILS_CARD_WIDTH,
        top: DETAILS_CARD_GAP,
        right: DETAILS_CARD_GAP
      }}
      className='border-border/60 bg-popover/85 absolute z-10 max-h-[calc(100%-1.5rem)] overflow-y-auto rounded-2xl border p-1.5 shadow-lg backdrop-blur-xl'
    >
      <DetailsContent threadId={threadId} />
    </aside>
  )
}

function DetailsContent({
  threadId,
  onNavigate
}: {
  threadId: string
  /** Called after a row moves the user elsewhere, so a popover can close. */
  onNavigate?: () => void
}) {
  const thread = useAppStore((state) =>
    state.threads.find((candidate) => candidate.id === threadId)
  )
  const root = useAppStore(
    (state) =>
      state.projects.find((project) => project.id === thread?.projectId)
        ?.workspaceRoot
  )
  const chatFolder = useChatFolder(thread?.projectId === null ? threadId : null)
  const pullRequest = useAppStore((state) => state.pullRequests[threadId])
  const editors = useDetailsStore((state) => state.editors)
  const changes = useDetailsStore((state) => state.changes[threadId])
  const loadEditors = useDetailsStore((state) => state.loadEditors)
  const loadChanges = useDetailsStore((state) => state.loadChanges)
  const openSurface = usePanelStore((state) => state.openSurface)
  const preferred = useEditorStore((state) => state.preferred)
  const setPreferred = useEditorStore((state) => state.setPreferred)
  const [error, setError] = useState<string | null>(null)
  const changeKey = thread
    ? `${thread.branch}|${thread.worktreePath}|${thread.status}|${thread.updatedAt}`
    : ''

  useEffect(() => {
    void loadEditors()
  }, [loadEditors])

  useEffect(() => {
    if (changeKey) void loadChanges(threadId)
  }, [threadId, changeKey, loadChanges])

  if (!thread) return null
  const path = thread.worktreePath ?? root ?? chatFolder ?? ''
  const folder = path.split(/[\\/]/).filter(Boolean).pop() ?? path
  const worktree = thread.worktreePath !== null
  const installed = editors ?? []
  const editor = openTarget(editors, preferred)

  const run = async (command: string, args: Record<string, unknown>) => {
    setError(null)
    try {
      await invoke(command, { threadId, ...args })
      return true
    } catch (failure) {
      setError(errorMessage(failure as CommandError))
      return false
    }
  }
  const openIn = (choice: Editor) => {
    setPreferred(choice)
    void run('open_in_editor', { editor: choice })
  }
  const show = (kind: 'pullRequest' | 'diff') => {
    openSurface(threadId, kind)
    onNavigate?.()
  }

  return (
    <div className='flex flex-col gap-0.5'>
      <div
        title={path}
        className='text-foreground/80 flex h-8 min-w-0 items-center gap-2.5 rounded-lg pr-1 pl-2.5 text-[13px] font-medium'
      >
        <HugeiconsIcon icon={Folder01Icon} strokeWidth={2} className={ICON} />
        <span className='min-w-0 truncate'>{folder}</span>
        {worktree ? (
          <span className='border-border text-muted-foreground shrink-0 rounded-full border px-1.5 text-[0.625rem] font-normal'>
            {thread.worktreeReleased
              ? m.worktree_released_badge()
              : m.worktree_badge()}
          </span>
        ) : null}
        <button
          type='button'
          aria-label={m.show_in_folder()}
          title={m.show_in_folder()}
          onClick={() => void revealItemInDir(path)}
          className='hover:bg-foreground/5 focus-visible:ring-ring ml-auto flex size-6 shrink-0 cursor-pointer items-center justify-center rounded-md outline-none focus-visible:ring-2'
        >
          <HugeiconsIcon
            icon={FolderOpenIcon}
            strokeWidth={2}
            className={ICON}
          />
        </button>
      </div>
      {editor ? (
        <SplitRow
          primary={
            <button
              type='button'
              onClick={() => openIn(editor)}
              className={SPLIT_PRIMARY}
            >
              <EditorIcon editor={editor} className='size-4' />
              <span className='truncate'>
                {m.open_in_editor({ editor: EDITOR_LABEL[editor]() })}
              </span>
            </button>
          }
          menuLabel={m.choose_editor()}
          menu={installed.map((choice) => (
            <DropdownMenuItem
              key={choice}
              onClick={() => openIn(choice)}
              className='gap-2.5'
            >
              <EditorIcon editor={choice} className='size-4' />
              {EDITOR_LABEL[choice]()}
              {choice === editor ? (
                <DropdownMenuShortcut>{OPEN_SHORTCUT()}</DropdownMenuShortcut>
              ) : null}
            </DropdownMenuItem>
          ))}
        />
      ) : null}
      {thread.branch || pullRequest || changes ? (
        <div className='bg-border/70 mx-2.5 my-1 h-px' />
      ) : null}
      {thread.branch ? (
        <BranchRow
          threadId={threadId}
          branch={thread.branch}
          worktree={worktree}
          working={thread.status === 'working'}
          run={run}
        />
      ) : null}
      {pullRequest ? (
        <button
          type='button'
          onClick={() => show('pullRequest')}
          className={ROW}
        >
          <PullRequestIcon state={pullRequest.state} className='size-4' />
          <span className='shrink-0 tabular-nums'>#{pullRequest.number}</span>
          <span className='text-muted-foreground min-w-0 flex-1 truncate font-normal'>
            {pullRequest.title}
          </span>
        </button>
      ) : null}
      {changes ? (
        <button type='button' onClick={() => show('diff')} className={ROW}>
          <HugeiconsIcon icon={FileDiffIcon} strokeWidth={2} className={ICON} />
          <span className='shrink-0'>{m.surface_diff()}</span>
          <span className='text-muted-foreground min-w-0 flex-1 truncate font-normal'>
            {m.diff_scope_uncommitted()}
          </span>
          <span className='shrink-0 text-xs tabular-nums'>
            <span className='text-pr-open'>+{changes.additions}</span>{' '}
            <span className='text-status-error'>−{changes.deletions}</span>
          </span>
        </button>
      ) : null}
      {error ? (
        <p role='alert' className='text-destructive px-2.5 pt-1 pb-1 text-xs'>
          {error}
        </p>
      ) : null}
    </div>
  )
}

/** A chat's own folder, which only the engine knows; `null` for any other thread. */
function useChatFolder(threadId: string | null) {
  const [folder, setFolder] = useState<string | null>(null)
  useEffect(() => {
    setFolder(null)
    if (!threadId) return
    let current = true
    invoke<string | null>('get_thread_folder', { threadId })
      .then((found) => {
        if (current) setFolder(found)
      })
      .catch(() => {})
    return () => {
      current = false
    }
  }, [threadId])
  return folder
}

/** The current branch; its menu lists the local branches, read only when it opens. */
function BranchRow({
  threadId,
  branch,
  worktree,
  working,
  run
}: {
  threadId: string
  branch: string
  worktree: boolean
  working: boolean
  run: (command: string, args: Record<string, unknown>) => Promise<boolean>
}) {
  const [branches, setBranches] = useState<string[] | null>(null)
  const others = branches?.filter((name) => name !== branch) ?? []

  return (
    <DropdownMenu
      onOpenChange={(open) => {
        if (!open) return
        invoke<string[]>('get_thread_branches', { threadId })
          .then(setBranches)
          .catch(() => setBranches([]))
      }}
    >
      <DropdownMenuTrigger
        disabled={working}
        render={
          <button
            type='button'
            title={working ? m.error_thread_busy() : m.switch_branch()}
            className={ROW}
          />
        }
      >
        <HugeiconsIcon
          icon={worktree ? GitForkIcon : GitBranchIcon}
          strokeWidth={2}
          className={ICON}
        />
        <span className='min-w-0 flex-1 truncate font-mono text-xs'>
          {branch}
        </span>
        <HugeiconsIcon
          icon={ArrowDown01Icon}
          strokeWidth={2}
          className={ICON}
        />
      </DropdownMenuTrigger>
      <DropdownMenuContent align='end' className='max-h-72'>
        <DropdownMenuGroup>
          <DropdownMenuLabel>{m.switch_branch()}</DropdownMenuLabel>
          {branches !== null && others.length === 0 ? (
            <p className='text-muted-foreground px-2 py-1.5 text-xs'>
              {m.no_other_branches()}
            </p>
          ) : null}
          {others.map((name) => (
            <DropdownMenuItem
              key={name}
              className='font-mono text-xs'
              onClick={() => void run('switch_branch', { branch: name })}
            >
              {name}
            </DropdownMenuItem>
          ))}
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

const SPLIT_PRIMARY =
  'flex h-8 min-w-0 flex-1 cursor-pointer items-center gap-2.5 rounded-s-lg px-2.5 text-left text-[13px] font-medium text-foreground/80 outline-none hover:bg-foreground/5 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset'

/** A row whose main part runs an action and whose arrow opens alternatives. */
function SplitRow({
  primary,
  menu,
  menuLabel
}: {
  primary: ReactNode
  /** `null` when there is nothing to choose from. */
  menu: ReactNode
  menuLabel: string
}) {
  const row = useRef<HTMLDivElement>(null)

  return (
    <div
      ref={row}
      className='group/split hover:bg-foreground/[0.02] flex items-center rounded-lg'
    >
      {primary}
      {menu ? (
        <>
          <span className='group-hover/split:bg-border/70 h-4 w-px shrink-0 bg-transparent' />
          <DropdownMenu>
            <DropdownMenuTrigger
              render={
                <button
                  type='button'
                  aria-label={menuLabel}
                  className='hover:bg-foreground/5 focus-visible:ring-ring flex h-8 w-8 shrink-0 cursor-pointer items-center justify-center rounded-e-lg outline-none focus-visible:ring-2 focus-visible:ring-inset'
                />
              }
            >
              <HugeiconsIcon
                icon={ArrowDown01Icon}
                strokeWidth={2}
                className={ICON}
              />
            </DropdownMenuTrigger>
            <DropdownMenuContent anchor={row} align='end'>
              {menu}
            </DropdownMenuContent>
          </DropdownMenu>
        </>
      ) : null}
    </div>
  )
}
