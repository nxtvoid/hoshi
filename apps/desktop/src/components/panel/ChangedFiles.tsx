import { cn } from '@hoshi/utils'
import { DiffView } from './DiffView'
import { m } from '@/paraglide/messages'
import { useEffect, useState } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import { FileIcon } from '@/components/files/FileIcon'
import type { DiffFile } from '@/bindings/DiffFile'
import type { FileDiff } from '@/bindings/FileDiff'
import { ArrowRight01Icon } from '@hugeicons/core-free-icons'
import type { DiffFileStatus } from '@/bindings/DiffFileStatus'

const STATUS_COLOR: Partial<Record<DiffFileStatus, string>> = {
  added: 'text-pr-open',
  untracked: 'text-pr-open',
  deleted: 'text-status-error'
}

/** Resolves to `null` when the file has no diff to show, such as a binary. */
export type LoadFileDiff = (file: DiffFile) => Promise<FileDiff | null>

type DiffState =
  | { kind: 'loading' }
  | { kind: 'ready'; value: FileDiff | null }
  | { kind: 'failed' }

/** Changed files that each open to their diff, loaded only when opened. */
export function ChangedFiles({
  files,
  loadDiff,
  reloadKey
}: {
  files: DiffFile[]
  loadDiff: LoadFileDiff
  /** A change reloads the diffs that are open. */
  reloadKey: string
}) {
  return (
    <ul className='flex flex-col'>
      {files.map((file) => (
        <FileEntry
          key={file.path}
          file={file}
          loadDiff={loadDiff}
          reloadKey={reloadKey}
        />
      ))}
    </ul>
  )
}

export function totalChanges(files: DiffFile[]) {
  return files.reduce(
    (sum, file) => ({
      additions: sum.additions + (file.additions ?? 0),
      deletions: sum.deletions + (file.deletions ?? 0)
    }),
    { additions: 0, deletions: 0 }
  )
}

function FileEntry({
  file,
  loadDiff,
  reloadKey
}: {
  file: DiffFile
  loadDiff: LoadFileDiff
  reloadKey: string
}) {
  const [open, setOpen] = useState(false)
  const slash = file.path.lastIndexOf('/')
  const folder = slash === -1 ? '' : file.path.slice(0, slash + 1)
  const name = file.path.slice(slash + 1)

  return (
    <li className='border-border/60 border-b [content-visibility:auto]'>
      <button
        type='button'
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
        className='hover:bg-accent/60 focus-visible:ring-ring flex w-full items-center gap-1.5 px-2 py-1.5 text-left text-xs outline-none focus-visible:ring-2 focus-visible:ring-inset'
      >
        <HugeiconsIcon
          icon={ArrowRight01Icon}
          strokeWidth={2.5}
          className={cn(
            'text-muted-foreground size-3 shrink-0 transition-transform',
            open && 'rotate-90'
          )}
        />
        <FileIcon path={file.path} className='size-3.5' />
        <span className='flex min-w-0 flex-1 items-baseline font-mono'>
          <span className='text-muted-foreground truncate'>{folder}</span>
          <span className={cn('shrink-0', STATUS_COLOR[file.status])}>
            {name}
          </span>
        </span>
        {file.additions === null ? (
          <span className='text-muted-foreground shrink-0'>
            {m.diff_binary()}
          </span>
        ) : (
          <span className='shrink-0 tabular-nums'>
            <span className='text-pr-open'>+{file.additions}</span>{' '}
            <span className='text-status-error'>−{file.deletions ?? 0}</span>
          </span>
        )}
      </button>
      {open ? (
        <FileDiffBlock file={file} loadDiff={loadDiff} reloadKey={reloadKey} />
      ) : null}
    </li>
  )
}

function FileDiffBlock({
  file,
  loadDiff,
  reloadKey
}: {
  file: DiffFile
  loadDiff: LoadFileDiff
  reloadKey: string
}) {
  const [diff, setDiff] = useState<DiffState>({ kind: 'loading' })

  useEffect(() => {
    let current = true
    loadDiff(file)
      .then((value) => current && setDiff({ kind: 'ready', value }))
      .catch(() => current && setDiff({ kind: 'failed' }))
    return () => {
      current = false
    }
  }, [loadDiff, file, reloadKey])

  if (diff.kind === 'loading') return null
  if (diff.kind === 'failed')
    return <PanelMessage>{m.diff_load_failed()}</PanelMessage>
  if (diff.value === null)
    return <PanelMessage>{m.diff_unavailable()}</PanelMessage>
  return (
    <>
      <DiffView text={diff.value.text} />
      {diff.value.truncated ? (
        <PanelMessage>{m.diff_truncated()}</PanelMessage>
      ) : null}
    </>
  )
}

export function PanelMessage({ children }: { children: string }) {
  return (
    <p className='text-muted-foreground px-3 py-4 text-center text-xs'>
      {children}
    </p>
  )
}
