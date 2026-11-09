import { useState } from 'react'
import { m } from '@/paraglide/messages'
import { invoke } from '@tauri-apps/api/core'
import { HugeiconsIcon } from '@hugeicons/react'
import { useAppStore } from '@/state/app-store'
import { choose, notify } from '@/state/dialog-store'
import { PencilEdit02Icon } from '@hugeicons/core-free-icons'
import type { CommandError } from '@/bindings/CommandError'
import { IconButton } from '@hoshi/ui/components/icon-button'
import { useComposerFillStore } from '@/state/composer-fill-store'
import type { MessageAttachment } from '@/bindings/MessageAttachment'

/**
 * Takes the thread back to before this message, which returns to the composer, as t3code's
 * "Edit from here". The files go back too when the user asks.
 */
export function RewindButton({
  messageId,
  text,
  attachments
}: {
  messageId: string
  text: string
  attachments: MessageAttachment[]
}) {
  const threadId = useAppStore((state) => state.selectedThreadId)
  const busy = useAppStore(
    (state) =>
      state.threads.find((thread) => thread.id === state.selectedThreadId)
        ?.activeTurnId != null
  )
  const [rewinding, setRewinding] = useState(false)

  const rewind = async () => {
    if (!threadId) return
    const answer = await choose({
      title: m.edit_from_here_title(),
      description: m.edit_from_here_description(),
      confirmLabel: m.revert_keep_changes(),
      alternativeLabel: m.revert_files_too(),
      tone: 'warning',
      icon: PencilEdit02Icon
    })
    if (!answer) return
    setRewinding(true)
    try {
      await invoke('rewind_thread', {
        threadId,
        messageId,
        revertFiles: answer === 'alternative'
      })
      useComposerFillStore.getState().request({ threadId, text, attachments })
    } catch (error) {
      await notify({
        title: m.rewind_failed(),
        description: rewindFailure(error as CommandError),
        confirmLabel: m.dialog_ok(),
        tone: 'warning'
      })
    } finally {
      setRewinding(false)
    }
  }

  return (
    <IconButton
      label={m.edit_from_here()}
      onClick={() => void rewind()}
      disabled={busy || rewinding}
      side='top'
      className='text-muted-foreground size-6'
    >
      <HugeiconsIcon
        icon={PencilEdit02Icon}
        strokeWidth={2}
        className='size-3.5'
      />
    </IconButton>
  )
}

function rewindFailure(error: CommandError | null) {
  switch (error?.code) {
    case 'noCheckpoint':
      return m.rewind_no_checkpoint()
    case 'threadBusy':
      return m.rewind_busy()
    case 'gitFailed':
      return error.detail
    default:
      return m.rewind_failed_detail()
  }
}
