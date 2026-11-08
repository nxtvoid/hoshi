import { m } from '@/paraglide/messages'
import { invoke } from '@tauri-apps/api/core'
import { useCallback, useEffect, useState } from 'react'
import type { DiffFile } from '@/bindings/DiffFile'
import type { FileDiff } from '@/bindings/FileDiff'
import { ChangedFiles, PanelMessage, totalChanges } from '../ChangedFiles'

type Files =
  | { kind: 'loading' }
  | { kind: 'ready'; value: DiffFile[] }
  | { kind: 'failed' }

/** The files the pull request changes, as the forge sees them. */
export function CodeTab({
  threadId,
  version
}: {
  threadId: string
  /** A change reloads the files. */
  version: string
}) {
  const [files, setFiles] = useState<Files>({ kind: 'loading' })

  useEffect(() => {
    let current = true
    invoke<DiffFile[] | null>('get_pull_request_files', { threadId })
      .then((value) => {
        if (current) {
          setFiles(value ? { kind: 'ready', value } : { kind: 'failed' })
        }
      })
      .catch(() => current && setFiles({ kind: 'failed' }))
    return () => {
      current = false
    }
  }, [threadId, version])

  const loadDiff = useCallback(
    (file: DiffFile) =>
      invoke<FileDiff | null>('get_pull_request_file_diff', {
        threadId,
        path: file.path
      }),
    [threadId]
  )

  if (files.kind === 'loading') return null
  if (files.kind === 'failed') {
    return <PanelMessage>{m.diff_load_failed()}</PanelMessage>
  }
  if (files.value.length === 0) {
    return <PanelMessage>{m.no_changes()}</PanelMessage>
  }
  const totals = totalChanges(files.value)
  return (
    <>
      <p className='text-muted-foreground border-border border-b px-3 py-1.5 text-xs tabular-nums'>
        {m.diff_file_count({ count: files.value.length })}{' '}
        <span className='text-pr-open'>+{totals.additions}</span>{' '}
        <span className='text-status-error'>−{totals.deletions}</span>
      </p>
      <ChangedFiles
        files={files.value}
        loadDiff={loadDiff}
        reloadKey={version}
      />
    </>
  )
}
