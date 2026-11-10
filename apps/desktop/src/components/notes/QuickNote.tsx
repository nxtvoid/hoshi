import { useEffect, useState } from 'react'
import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { saveNote, useNotesStore } from '@/state/notes-store'
import { useAppStore } from '@/state/app-store'
import { Kbd } from '@hoshi/ui/components/kbd'
import { useScopeProjectId } from '@/state/use-scope'
import { Button } from '@hoshi/ui/components/button'
import { Textarea } from '@hoshi/ui/components/textarea'
import { useQuickNoteStore } from '@/state/quick-note-store'
import { isQuickNoteShortcut } from './quick-note-shortcut'
import { Dialog, DialogContent, DialogTitle } from '@hoshi/ui/components/dialog'

/** The shortcut that takes a note from anywhere, and the dialog it opens. */
export function QuickNote() {
  const open = useQuickNoteStore((state) => state.open)
  const setOpen = useQuickNoteStore((state) => state.setOpen)

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!isQuickNoteShortcut(event)) return
      event.preventDefault()
      setOpen(!useQuickNoteStore.getState().open)
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [setOpen])

  return open ? <QuickNoteDialog onClose={() => setOpen(false)} /> : null
}

function QuickNoteDialog({ onClose }: { onClose: () => void }) {
  const projectId = useScopeProjectId()
  const projectTitle = useAppStore(
    (state) => state.projects.find((project) => project.id === projectId)?.title
  )
  const [text, setText] = useState('')
  const [general, setGeneral] = useState(false)
  const [saving, setSaving] = useState(false)
  const written = text.trim() !== ''

  const save = async () => {
    if (!written || saving) return
    setSaving(true)
    try {
      await saveNote(
        crypto.randomUUID(),
        general ? null : projectId,
        text.trim()
      )
      useNotesStore.getState().captured()
      onClose()
    } catch (error) {
      console.error('hoshi: failed to save a note', error)
      setSaving(false)
    }
  }

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent
        showCloseButton={false}
        className='top-[18vh] translate-y-0 gap-0 overflow-hidden p-0 sm:max-w-lg'
      >
        <DialogTitle className='sr-only'>{m.notes_new()}</DialogTitle>
        <Textarea
          autoFocus
          value={text}
          rows={4}
          placeholder={m.notes_placeholder()}
          aria-label={m.notes_placeholder()}
          onChange={(event) => setText(event.target.value)}
          onKeyDown={(event) => {
            if (event.key !== 'Enter' || event.shiftKey) return
            if (event.nativeEvent.isComposing) return
            event.preventDefault()
            void save()
          }}
          className='max-h-[50vh] min-h-28 resize-none rounded-none border-0 px-4 py-3 leading-relaxed focus-visible:ring-0 dark:bg-transparent'
        />
        <div className='border-border flex items-center gap-2 border-t px-2 py-1.5'>
          {projectId !== null && projectTitle ? (
            <div className='bg-muted flex min-w-0 rounded-lg p-0.5 text-xs'>
              <ScopeButton active={!general} onClick={() => setGeneral(false)}>
                {projectTitle}
              </ScopeButton>
              <ScopeButton active={general} onClick={() => setGeneral(true)}>
                {m.notes_general()}
              </ScopeButton>
            </div>
          ) : (
            <span className='text-muted-foreground px-2 text-xs'>
              {m.notes_general()}
            </span>
          )}
          <Button
            size='sm'
            className='ms-auto'
            disabled={!written || saving}
            onClick={() => void save()}
          >
            {m.save()}
            <Kbd className='bg-primary-foreground/15 text-primary-foreground'>
              ↵
            </Kbd>
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}

function ScopeButton({
  active,
  onClick,
  children
}: {
  active: boolean
  onClick: () => void
  children: React.ReactNode
}) {
  return (
    <button
      type='button'
      aria-pressed={active}
      onClick={onClick}
      className={cn(
        'focus-visible:ring-ring max-w-40 truncate rounded-md px-2 py-1 outline-none focus-visible:ring-2',
        active
          ? 'bg-background text-foreground shadow-xs'
          : 'text-muted-foreground hover:text-foreground'
      )}
    >
      {children}
    </button>
  )
}
