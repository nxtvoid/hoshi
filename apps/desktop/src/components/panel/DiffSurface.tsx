import { m } from '@/paraglide/messages'
import { invoke } from '@tauri-apps/api/core'
import { useAppStore } from '@/state/app-store'
import { HugeiconsIcon } from '@hugeicons/react'
import { usePanelStore } from '@/state/panel-store'
import type { Surface } from '@/state/panel-surfaces'
import type { DiffFile } from '@/bindings/DiffFile'
import type { FileDiff } from '@/bindings/FileDiff'
import { useCallback, useEffect, useState } from 'react'
import { Button } from '@hoshi/ui/components/button'
import type { CommandError } from '@/bindings/CommandError'
import { IconButton } from '@hoshi/ui/components/icon-button'
import { Refresh01Icon } from '@hugeicons/core-free-icons'
import { ChangedFiles, PanelMessage, totalChanges } from './ChangedFiles'

type ScopeOption = { id: DiffSurfaceState['scope']; label: () => string }

const SCOPES: ScopeOption[] = [
  { id: 'uncommitted', label: m.diff_scope_uncommitted },
  { id: 'branch', label: m.diff_scope_branch }
]

const TURN_SCOPE: ScopeOption = { id: 'turn', label: m.diff_scope_turn }

type Files =
  | { kind: 'loading' }
  | { kind: 'ready'; value: DiffFile[] }
  | { kind: 'failed'; error: CommandError | null }

/** Changes a thread's turns or tool calls may have made; read again after each turn. */
function useChangeKey(threadId: string) {
  return useAppStore((state) => {
    const thread = state.threads.find((candidate) => candidate.id === threadId)
    return thread
      ? `${thread.updatedAt}|${thread.status}|${thread.branch}|${thread.worktreePath}`
      : ''
  })
}

type DiffSurfaceState = Extract<Surface, { kind: 'diff' }>

/** What the thread's working tree changed, uncommitted or since the branch began. */
export function DiffSurface({
  threadId,
  surface
}: {
  threadId: string
  surface: DiffSurfaceState
}) {
  const { scope, turnId, focusPath } = surface
  const updateSurface = usePanelStore((state) => state.updateSurface)
  const setScope = (next: DiffSurfaceState['scope']) =>
    updateSurface<DiffSurfaceState>(threadId, surface.id, (current) => ({
      ...current,
      scope: next,
      focusPath: undefined
    }))
  const scopes = turnId ? [...SCOPES, TURN_SCOPE] : SCOPES
  const changeKey = useChangeKey(threadId)
  const [refreshes, setRefreshes] = useState(0)
  const [files, setFiles] = useState<Files>({ kind: 'loading' })
  const reloadKey = `${changeKey}|${refreshes}`

  useEffect(() => {
    let current = true
    const listing =
      scope === 'turn'
        ? invoke<DiffFile[]>('get_turn_diff_files', { threadId, turnId })
        : invoke<DiffFile[]>('get_thread_diff_files', { threadId, scope })
    listing
      .then((value) => current && setFiles({ kind: 'ready', value }))
      .catch(
        (error: CommandError) => current && setFiles({ kind: 'failed', error })
      )
    return () => {
      current = false
    }
  }, [threadId, scope, turnId, reloadKey])

  const loadDiff = useCallback(
    (file: DiffFile) =>
      scope === 'turn'
        ? invoke<FileDiff>('get_turn_file_diff', { threadId, turnId, file })
        : invoke<FileDiff>('get_thread_file_diff', { threadId, scope, file }),
    [threadId, scope, turnId]
  )
  const totals = files.kind === 'ready' ? totalChanges(files.value) : null

  return (
    <>
      <div className='border-border flex shrink-0 items-center gap-1 border-b px-2 py-1.5'>
        {scopes.map(({ id, label }) => (
          <Button
            key={id}
            size='xs'
            variant={scope === id ? 'outline' : 'ghost'}
            aria-pressed={scope === id}
            onClick={() => setScope(id)}
          >
            {label()}
          </Button>
        ))}
        {totals && files.kind === 'ready' ? (
          <span className='text-muted-foreground ml-auto text-xs tabular-nums'>
            {m.diff_file_count({ count: files.value.length })}{' '}
            <span className='text-pr-open'>+{totals.additions}</span>{' '}
            <span className='text-status-error'>−{totals.deletions}</span>
          </span>
        ) : (
          <span className='ml-auto' />
        )}
        <IconButton
          label={m.refresh()}
          onClick={() => setRefreshes((count) => count + 1)}
        >
          <HugeiconsIcon icon={Refresh01Icon} strokeWidth={2} />
        </IconButton>
      </div>
      <div className='min-h-0 flex-1 overflow-y-auto'>
        {files.kind === 'loading' ? null : files.kind === 'failed' ? (
          <PanelMessage>
            {files.error?.code === 'notAGitRepository'
              ? m.diff_not_a_repository()
              : m.diff_load_failed()}
          </PanelMessage>
        ) : files.value.length === 0 ? (
          <PanelMessage>{m.no_changes()}</PanelMessage>
        ) : (
          <ChangedFiles
            key={`${scope}:${turnId}:${focusPath}`}
            files={files.value}
            focusPath={focusPath}
            loadDiff={loadDiff}
            reloadKey={reloadKey}
          />
        )}
      </div>
    </>
  )
}
