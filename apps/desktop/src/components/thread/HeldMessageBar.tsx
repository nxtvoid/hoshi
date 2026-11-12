import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { CHAT_LANE } from '@/lib/chat-lane'
import { invoke } from '@tauri-apps/api/core'
import { HugeiconsIcon } from '@hugeicons/react'
import { Button } from '@hoshi/ui/components/button'
import type { HeldMessage } from '@/bindings/HeldMessage'
import { useImageStore } from '@/state/image-store'
import { useHeldMessageStore } from '@/state/held-message-store'
import { useComposerFillStore } from '@/state/composer-fill-store'
import {
  ArrowUp02Icon,
  Cancel01Icon,
  PencilEdit01Icon
} from '@hugeicons/core-free-icons'

/**
 * Above the composer while the thread's conversation is compacted: the message that goes
 * once it is, which the user can send at once, take back to edit, or drop.
 */
export function HeldMessageBar({ threadId }: { threadId: string }) {
  const held = useHeldMessageStore((state) => state.byThread[threadId])
  if (!held) return null
  const images = held.attachments.filter(
    (attachment) => attachment.type === 'image'
  ).length
  const files = held.attachments.filter(
    (attachment) => attachment.type === 'file'
  ).length

  const takeBack = async (discard: boolean) => {
    const taken = await invoke<HeldMessage | null>('take_held_message', {
      threadId,
      discard
    }).catch((error) => {
      console.error('hoshi: failed to take back a held message', error)
      return null
    })
    if (taken && !discard) {
      await Promise.all(
        taken.attachments.map((attachment) =>
          attachment.type === 'image'
            ? useImageStore.getState().load(attachment.id, attachment.mimeType)
            : null
        )
      )
      useComposerFillStore.getState().request({
        threadId,
        text: taken.text,
        attachments: taken.attachments
      })
    }
  }

  return (
    <div className={cn(CHAT_LANE, 'mb-2 flex flex-col items-end gap-1')}>
      <div className='border-muted-foreground/40 max-w-[85%] rounded-xl border border-dashed px-3 py-2 text-sm'>
        {held.text ? (
          <p className='line-clamp-3 break-words whitespace-pre-wrap'>
            {held.text}
          </p>
        ) : null}
        {images > 0 ? (
          <p className='text-muted-foreground text-xs'>
            {m.held_message_images({ count: images })}
          </p>
        ) : null}
        {files > 0 ? (
          <p className='text-muted-foreground text-xs'>
            {m.held_message_files({ count: files })}
          </p>
        ) : null}
      </div>
      <div className='text-muted-foreground flex items-center gap-0.5 text-xs'>
        <Button
          size='xs'
          variant='ghost'
          title={m.held_message_send_now_hint()}
          className='text-foreground'
          onClick={() =>
            void invoke('send_held_message_now', { threadId }).catch((error) =>
              console.error('hoshi: failed to send a held message', error)
            )
          }
        >
          <HugeiconsIcon icon={ArrowUp02Icon} strokeWidth={2} />
          {m.held_message_send_now()}
        </Button>
        <Button
          size='xs'
          variant='ghost'
          className='text-muted-foreground hover:text-foreground'
          onClick={() => void takeBack(false)}
        >
          <HugeiconsIcon icon={PencilEdit01Icon} strokeWidth={2} />
          {m.held_message_edit()}
        </Button>
        <Button
          size='xs'
          variant='ghost'
          className='text-muted-foreground hover:text-foreground'
          onClick={() => void takeBack(true)}
        >
          <HugeiconsIcon icon={Cancel01Icon} strokeWidth={2} />
          {m.held_message_discard()}
        </Button>
      </div>
    </div>
  )
}
