import { useCallback, useEffect, useRef, useState } from 'react'
import { m } from '@/paraglide/messages'
import { NotesHeader } from './NotesHeader'
import { NotePlaceIcon } from './NotesProjectPicker'
import type { Note } from '@/bindings/Note'
import type { JSONContent } from '@tiptap/core'
import { toMarkdown } from '@/lib/composer-markdown'
import { HugeiconsIcon } from '@hugeicons/react'
import { confirm } from '@/state/dialog-store'
import { noteReference } from '@/lib/note-reference'
import type { Node as ProseMirrorNode } from '@tiptap/pm/model'
import { NoteTextEditor, type NoteTextEditorHandle } from './NoteTextEditor'
import {
  changedStretch,
  noteDocument,
  replaceInDocument
} from '@/lib/note-document'
import { useAppStore } from '@/state/app-store'
import { Button } from '@hoshi/ui/components/button'
import { IconButton } from '@hoshi/ui/components/icon-button'
import { VaultSecretDialog } from '@/components/vault/VaultSecretDialog'
import {
  Alert02Icon,
  Delete02Icon,
  FolderTransferIcon,
  Link01Icon,
  MoreHorizontalIcon,
  Tick02Icon
} from '@hugeicons/core-free-icons'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger
} from '@hoshi/ui/components/dropdown-menu'
import {
  deleteNote,
  ignoreNoteCredential,
  moveNoteCredentialToVault,
  readNote,
  saveNote
} from '@/state/notes-store'

/** Quiet time after the last key before the note is saved. */
const SAVE_DELAY_MS = 400
const COPIED_MS = 1500

/** A note's title names the secret moved out of it, unless the secret was in the title. */
function suggestedName(note: Note | null) {
  return note && !note.title.includes('•') ? note.title : ''
}

type NoteEditorProps = {
  /** A new id opens an empty note, kept once it has text. */
  noteId: string
  /** Where a note not written yet goes: a project, or `null` for the general ones. */
  newProjectId: string | null
  /** The user moved the note to a project, or to the general ones for `null`. */
  onMoved: (projectId: string | null) => void
  onDeleted: () => void
}

/** What a note holds: its markdown, the document its editor shows, and where it belongs. */
type Kept = { text: string; document: string | null; projectId: string | null }

/** The document a note opens with: the one its editor left, or its text read as one. */
function documentOf(note: { text: string; document: string | null }) {
  if (note.document) {
    try {
      return JSON.parse(note.document) as JSONContent
    } catch {
      return noteDocument(note.text)
    }
  }
  return noteDocument(note.text)
}

/**
 * One note as rich text, saved as it is typed: as markdown, which is what hoshi reads, and
 * as the document the editor shows. An emptied note is deleted on leaving.
 */
export function NoteEditor({
  noteId,
  newProjectId,
  onMoved,
  onDeleted
}: NoteEditorProps) {
  const [projectId, setProjectId] = useState(newProjectId)
  /** What the editor opens with; `null` until the note is read. */
  const [opening, setOpening] = useState<JSONContent | null>(null)
  const [empty, setEmpty] = useState(true)
  const [note, setNote] = useState<Note | null>(null)
  const [movingToVault, setMovingToVault] = useState(false)
  const editor = useRef<NoteTextEditorHandle>(null)
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const chain = useRef<Promise<void>>(Promise.resolve())
  const current = useRef<Kept>({ text: '', document: null, projectId })
  /** The editor's document since the last edit, not yet written out into `current`. */
  const edited = useRef<ProseMirrorNode | null>(null)
  /** What the store holds: `null` while the note isn't in it. */
  const stored = useRef<Kept | null>(null)

  const write = useCallback(() => {
    chain.current = chain.current.then(async () => {
      if (edited.current) {
        const document = edited.current.toJSON() as JSONContent
        edited.current = null
        current.current = {
          ...current.current,
          text: toMarkdown(document),
          document: JSON.stringify(document)
        }
      }
      const next = current.current
      const kept = stored.current
      if (kept === null && next.text.trim() === '') return
      if (
        kept?.text === next.text &&
        kept.document === next.document &&
        kept.projectId === next.projectId
      ) {
        return
      }
      try {
        const saved = await saveNote(
          noteId,
          next.projectId,
          next.text,
          next.document
        )
        stored.current = next
        setNote(saved)
      } catch (error) {
        console.error('hoshi: failed to save a note', error)
      }
    })
    return chain.current
  }, [noteId])

  const flush = useCallback(() => {
    if (timer.current) clearTimeout(timer.current)
    timer.current = null
    return write()
  }, [write])

  useEffect(() => {
    let cancelled = false
    void readNote(noteId).then((found) => {
      if (cancelled) return
      if (found) {
        stored.current = {
          text: found.text,
          document: found.document,
          projectId: found.projectId
        }
        current.current = { ...stored.current }
        setProjectId(found.projectId)
      }
      setNote(found)
      setEmpty((found?.text ?? '').trim() === '')
      setOpening(documentOf(found ?? { text: '', document: null }))
    })
    return () => {
      cancelled = true
      void flush().then(() => {
        if (stored.current !== null && current.current.text.trim() === '') {
          return deleteNote(noteId).catch(() => {})
        }
      })
    }
  }, [noteId, flush])

  const edit = (document: ProseMirrorNode, nowEmpty: boolean) => {
    edited.current = document
    setEmpty(nowEmpty)
    if (timer.current) clearTimeout(timer.current)
    timer.current = setTimeout(() => void write(), SAVE_DELAY_MS)
  }

  const move = (next: string | null) => {
    setProjectId(next)
    current.current = { ...current.current, projectId: next }
    void flush()
    onMoved(next)
  }

  /**
   * Shows the note as the vault left it, with the secret's name where its value was. The
   * editor's document held the value too, so it is written again without it.
   */
  const takeWithoutCredential = (updated: Note) => {
    const before = current.current
    const { from, to } = changedStretch(before.text, updated.text)
    const patched = replaceInDocument(documentOf(before), from, to)
    const shown =
      patched && toMarkdown(patched) === updated.text
        ? patched
        : noteDocument(updated.text)
    editor.current?.setDocument(shown)
    stored.current = {
      text: updated.text,
      document: updated.document,
      projectId: updated.projectId
    }
    current.current = { ...stored.current, document: JSON.stringify(shown) }
    setNote(updated)
    void write()
  }

  const erase = async () => {
    const confirmed = await confirm({
      title: m.notes_delete_confirm_title(),
      description: m.notes_delete_confirm_description(),
      confirmLabel: m.notes_delete(),
      tone: 'destructive'
    })
    if (!confirmed) return
    if (timer.current) clearTimeout(timer.current)
    await chain.current
    edited.current = null
    current.current = { ...current.current, text: '' }
    if (stored.current !== null) {
      await deleteNote(noteId).catch((error) =>
        console.error('hoshi: failed to delete a note', error)
      )
      stored.current = null
    }
    onDeleted()
  }

  const notASecret = async () => {
    await flush()
    try {
      setNote(await ignoreNoteCredential(noteId, 0))
    } catch (error) {
      console.error('hoshi: failed to dismiss a credential', error)
    }
  }

  const [credential, ...others] = note?.credentials ?? []

  return (
    <>
      <NotesHeader
        actions={
          <>
            <CopyReferenceButton
              noteId={noteId}
              disabled={empty}
              flush={flush}
            />
            <DropdownMenu>
              <DropdownMenuTrigger
                render={
                  <Button
                    variant='ghost'
                    size='icon-sm'
                    aria-label={m.notes_more()}
                  />
                }
              >
                <HugeiconsIcon icon={MoreHorizontalIcon} strokeWidth={2} />
              </DropdownMenuTrigger>
              <DropdownMenuContent align='end' className='w-44'>
                <MoveMenu projectId={projectId} onMove={move} />
                <DropdownMenuItem
                  variant='destructive'
                  onClick={() => void erase()}
                >
                  <HugeiconsIcon icon={Delete02Icon} strokeWidth={2} />
                  {m.notes_delete()}
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </>
        }
      >
        <h1
          data-tauri-drag-region
          title={note?.title || undefined}
          className='max-w-64 min-w-0 truncate text-sm font-medium'
        >
          {note ? note.title || m.notes_untitled() : m.notes_new()}
        </h1>
      </NotesHeader>
      <div className='flex min-h-0 flex-1 flex-col overflow-hidden'>
        {opening ? (
          <NoteTextEditor
            ref={editor}
            initialDocument={opening}
            placeholder={m.notes_placeholder()}
            onChange={edit}
            onBlur={() => void flush()}
          />
        ) : (
          <div className='flex-1' />
        )}
        {credential ? (
          <div className='mx-auto w-full max-w-3xl shrink-0 px-4 pb-4'>
            <div className='border-border bg-status-waiting/8 flex flex-wrap items-center gap-x-3 gap-y-2 rounded-xl border px-3 py-2.5'>
              <p className='flex min-w-0 flex-1 items-start gap-1.5 text-xs'>
                <HugeiconsIcon
                  icon={Alert02Icon}
                  strokeWidth={2}
                  className='text-status-waiting mt-px size-3.5 shrink-0'
                />
                <span className='min-w-0'>
                  {m.notes_credential_found()}{' '}
                  <code className='bg-foreground/10 rounded px-1 font-mono text-[0.6875rem]'>
                    {credential}
                  </code>
                  {others.length > 0
                    ? ` ${m.notes_credential_more({ count: others.length })}`
                    : null}
                  <span className='text-muted-foreground block'>
                    {m.notes_credential_hint()}
                  </span>
                </span>
              </p>
              <div className='flex shrink-0 gap-1.5'>
                <Button
                  size='xs'
                  variant='outline'
                  onClick={() => void notASecret()}
                >
                  {m.notes_not_a_secret()}
                </Button>
                <Button size='xs' onClick={() => setMovingToVault(true)}>
                  {m.notes_move_to_vault()}
                </Button>
              </div>
            </div>
          </div>
        ) : null}
      </div>
      {movingToVault && credential ? (
        <VaultSecretDialog
          title={m.notes_move_to_vault()}
          valueHint={credential}
          initial={{ name: suggestedName(note), projectId }}
          onSubmit={async (secret) => {
            await flush()
            takeWithoutCredential(
              await moveNoteCredentialToVault(noteId, 0, secret)
            )
          }}
          onClose={() => setMovingToVault(false)}
        />
      ) : null}
    </>
  )
}

/** Moves the note to another project, or to the general ones. */
function MoveMenu({
  projectId,
  onMove
}: {
  projectId: string | null
  onMove: (projectId: string | null) => void
}) {
  const projects = useAppStore((state) => state.projects)
  const places = [
    { id: null as string | null, title: m.notes_general() },
    ...projects.map(({ id, title }) => ({ id: id as string | null, title }))
  ].filter((place) => place.id !== projectId)

  return (
    <DropdownMenuSub>
      <DropdownMenuSubTrigger>
        <HugeiconsIcon icon={FolderTransferIcon} strokeWidth={2} />
        {m.notes_move_to()}
      </DropdownMenuSubTrigger>
      <DropdownMenuSubContent className='max-h-72 w-52 overflow-y-auto'>
        {places.map((place) => (
          <DropdownMenuItem
            key={place.id ?? ''}
            onClick={() => onMove(place.id)}
          >
            <NotePlaceIcon projectId={place.id} title={place.title} />
            <span className='min-w-0 truncate'>{place.title}</span>
          </DropdownMenuItem>
        ))}
      </DropdownMenuSubContent>
    </DropdownMenuSub>
  )
}

/**
 * Copies the note's reference: pasted into a message, it attaches the note. The note is
 * saved first, so the reference leads somewhere.
 */
function CopyReferenceButton({
  noteId,
  disabled,
  flush
}: {
  noteId: string
  disabled: boolean
  flush: () => Promise<void>
}) {
  const [copied, setCopied] = useState(false)
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current)
    },
    []
  )

  const copy = async () => {
    try {
      await flush()
      await navigator.clipboard.writeText(noteReference(noteId))
      if (timer.current) clearTimeout(timer.current)
      setCopied(true)
      timer.current = setTimeout(() => setCopied(false), COPIED_MS)
    } catch (error) {
      console.error('hoshi: failed to copy a note reference', error)
    }
  }

  return (
    <IconButton
      label={copied ? m.notes_reference_copied() : m.notes_copy_reference()}
      disabled={disabled}
      onClick={() => void copy()}
    >
      <HugeiconsIcon icon={copied ? Tick02Icon : Link01Icon} strokeWidth={2} />
    </IconButton>
  )
}
