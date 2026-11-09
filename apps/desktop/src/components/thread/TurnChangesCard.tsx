import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { useMemo, useState } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import { useAppStore } from '@/state/app-store'
import { usePanelStore } from '@/state/panel-store'
import type { TurnChanges } from '@/lib/tool-calls'
import { FileIcon } from '@/components/files/FileIcon'
import { Button } from '@hoshi/ui/components/button'
import { IconButton } from '@hoshi/ui/components/icon-button'
import { totalChanges } from '@/components/panel/ChangedFiles'
import { buildFileTree, hasFolders, type FileTreeNode } from '@/lib/file-tree'
import {
  isChangesCardOpen,
  useTurnChangesStore
} from '@/state/turn-changes-store'
import {
  ArrowRight01Icon,
  FileDiffIcon,
  Folder01Icon,
  FolderOpenIcon,
  UnfoldLessIcon,
  UnfoldMoreIcon
} from '@hugeicons/core-free-icons'

const TREE_ROW =
  'group hover:bg-accent/60 focus-visible:ring-ring flex h-7 w-full items-center gap-2 rounded-md py-1.5 pr-2 text-left outline-none focus-visible:ring-2 focus-visible:ring-inset'

type OpenFile = (path?: string) => void

/** What a finished turn changed, as folders and files; each opens the turn's diff. */
export function TurnChangesCard({ changes }: { changes: TurnChanges }) {
  const threadId = useAppStore((state) => state.selectedThreadId)
  const openTurnDiff = usePanelStore((state) => state.openTurnDiff)
  const tree = useMemo(() => buildFileTree(changes.files), [changes.files])
  const totals = useMemo(() => totalChanges(changes.files), [changes.files])
  const [allOpen, setAllOpen] = useState(false)
  const chosen = useTurnChangesStore((state) => state.open[changes.turnId])
  const setOpen = useTurnChangesStore((state) => state.setOpen)
  const open = isChangesCardOpen(chosen, changes.files.length)
  const openFile: OpenFile = (path) => {
    if (threadId) openTurnDiff(threadId, changes.turnId, path)
  }
  const folders = hasFolders(tree)

  return (
    <section className='bg-secondary dark:bg-card @container/changes mt-4 rounded-lg'>
      <header
        className={cn(
          'bg-secondary dark:bg-card sticky top-2 z-10 flex items-center justify-between gap-2 py-2 pr-3 pl-1.5',
          open ? 'rounded-t-lg' : 'rounded-lg'
        )}
      >
        <button
          type='button'
          aria-expanded={open}
          onClick={() => setOpen(changes.turnId, !open)}
          className='text-foreground focus-visible:ring-ring group flex min-w-0 items-center gap-x-1.5 rounded-md px-1.5 text-xs font-medium outline-none focus-visible:ring-2'
        >
          <HugeiconsIcon
            icon={ArrowRight01Icon}
            strokeWidth={2}
            className={cn(
              'text-muted-foreground group-hover:text-foreground size-3.5 shrink-0 transition-transform',
              open && 'rotate-90'
            )}
          />
          <span className='flex items-center gap-x-3'>
            {m.turn_changed_files({ count: changes.files.length })}
            <Stat additions={totals.additions} deletions={totals.deletions} />
          </span>
        </button>
        <span className='flex items-center gap-1'>
          {open && folders ? (
            <IconButton
              label={
                allOpen ? m.collapse_all_folders() : m.expand_all_folders()
              }
              onClick={() => setAllOpen(!allOpen)}
              className='text-muted-foreground size-6'
            >
              <HugeiconsIcon
                icon={allOpen ? UnfoldLessIcon : UnfoldMoreIcon}
                strokeWidth={2}
                className='size-3'
              />
            </IconButton>
          ) : null}
          <Button
            size='xs'
            variant='ghost'
            aria-label={m.open_diff()}
            title={m.open_full_diff()}
            onClick={() => openFile(changes.files[0]?.path)}
            className='text-muted-foreground'
          >
            <HugeiconsIcon
              icon={FileDiffIcon}
              strokeWidth={2}
              className='size-3'
            />
            <span className='hidden @[24rem]/changes:inline'>
              {m.open_diff()}
            </span>
          </Button>
        </span>
      </header>
      {open ? (
        <ul
          key={String(allOpen)}
          className='max-h-80 overflow-y-auto overscroll-contain p-2'
        >
          <TreeRows
            nodes={tree}
            depth={0}
            allOpen={allOpen}
            indentFiles={folders}
            openFile={openFile}
          />
        </ul>
      ) : null}
    </section>
  )
}

function TreeRows({
  nodes,
  depth,
  allOpen,
  indentFiles,
  openFile
}: {
  nodes: FileTreeNode[]
  depth: number
  allOpen: boolean
  /** Files leave room for a folder's chevron, so names line up. */
  indentFiles: boolean
  openFile: OpenFile
}) {
  return nodes.map((node) =>
    node.kind === 'folder' ? (
      <FolderRow
        key={node.path}
        node={node}
        depth={depth}
        allOpen={allOpen}
        openFile={openFile}
      />
    ) : (
      <li key={node.path}>
        <button
          type='button'
          onClick={() => openFile(node.path)}
          className={TREE_ROW}
          style={{ paddingLeft: indent(depth) }}
        >
          {indentFiles || depth > 0 ? (
            <span className='size-3.5 shrink-0' />
          ) : null}
          <FileIcon path={node.path} className='size-3.5 shrink-0' />
          <span className='text-foreground/85 group-hover:text-foreground min-w-0 truncate font-mono text-xs'>
            {node.name}
          </span>
          <Stat
            additions={node.file.additions ?? 0}
            deletions={node.file.deletions ?? 0}
            className='ml-auto'
          />
        </button>
      </li>
    )
  )
}

function FolderRow({
  node,
  depth,
  allOpen,
  openFile
}: {
  node: Extract<FileTreeNode, { kind: 'folder' }>
  depth: number
  allOpen: boolean
  openFile: OpenFile
}) {
  const [open, setOpen] = useState(allOpen)
  return (
    <li>
      <button
        type='button'
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className={TREE_ROW}
        style={{ paddingLeft: indent(depth) }}
      >
        <HugeiconsIcon
          icon={ArrowRight01Icon}
          strokeWidth={2}
          className={cn(
            'text-muted-foreground size-3.5 shrink-0 transition-transform',
            open && 'rotate-90'
          )}
        />
        <HugeiconsIcon
          icon={open ? FolderOpenIcon : Folder01Icon}
          strokeWidth={2}
          className='text-muted-foreground size-3.5 shrink-0'
        />
        <span className='text-muted-foreground min-w-0 truncate font-mono text-[0.6875rem]'>
          {node.name}
        </span>
        <Stat
          additions={node.additions}
          deletions={node.deletions}
          className='ml-auto'
        />
      </button>
      {open ? (
        <ul>
          <TreeRows
            nodes={node.children}
            depth={depth + 1}
            allOpen={allOpen}
            indentFiles
            openFile={openFile}
          />
        </ul>
      ) : null}
    </li>
  )
}

function indent(depth: number) {
  return 8 + depth * 14
}

function Stat({
  additions,
  deletions,
  className
}: {
  additions: number
  deletions: number
  className?: string
}) {
  if (additions === 0 && deletions === 0) return null
  return (
    <span
      className={cn(
        'shrink-0 font-mono text-[0.6875rem] tabular-nums',
        className
      )}
      title={m.diff_stat_label({ additions, deletions })}
    >
      <span className='text-pr-open'>+{additions}</span>{' '}
      <span className='text-status-error'>−{deletions}</span>
    </span>
  )
}
