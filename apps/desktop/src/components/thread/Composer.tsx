import { useState } from 'react'
import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { useAppStore } from '@/state/app-store'
import { HugeiconsIcon } from '@hugeicons/react'
import { Button } from '@hoshi/ui/components/button'
import { Textarea } from '@hoshi/ui/components/textarea'
import {
  ArrowUp02Icon,
  Folder01Icon,
  GitBranchIcon,
  StopIcon
} from '@hugeicons/core-free-icons'

type ComposerProps = {
  working: boolean
  /** Centered on an empty thread, with room to write; docked below a transcript. */
  variant: 'hero' | 'docked'
}

export function Composer({ working, variant }: ComposerProps) {
  const [text, setText] = useState('')
  const [sending, setSending] = useState(false)
  const sendMessage = useAppStore((state) => state.sendMessage)
  const interruptTurn = useAppStore((state) => state.interruptTurn)
  const canSend = text.trim() !== '' && !working && !sending
  const hero = variant === 'hero'

  const submit = async () => {
    if (!canSend) return
    setSending(true)
    const sent = await sendMessage(text)
    setSending(false)
    if (sent) setText('')
  }

  const action = working ? (
    <Button
      type='button'
      size='icon-sm'
      variant='secondary'
      aria-label={m.stop_turn()}
      className='shrink-0 rounded-full'
      onClick={() => void interruptTurn()}
    >
      <HugeiconsIcon icon={StopIcon} strokeWidth={2} />
    </Button>
  ) : (
    <Button
      type='submit'
      size='icon-sm'
      aria-label={m.send_message()}
      className='shrink-0 rounded-full'
      disabled={!canSend}
    >
      <HugeiconsIcon icon={ArrowUp02Icon} strokeWidth={2} />
    </Button>
  )

  return (
    <div className='mx-auto flex w-full max-w-3xl flex-col gap-1.5'>
      <form
        className={cn(
          'border-border bg-card focus-within:border-ring/60 flex rounded-2xl border shadow-xs transition-colors',
          hero ? 'flex-col gap-1 p-2' : 'items-end gap-2 py-1.5 pr-2 pl-1'
        )}
        onSubmit={(event) => {
          event.preventDefault()
          void submit()
        }}
      >
        <Textarea
          autoFocus
          rows={1}
          value={text}
          onChange={(event) => setText(event.currentTarget.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter' && !event.shiftKey) {
              event.preventDefault()
              void submit()
            }
          }}
          placeholder={m.composer_placeholder()}
          aria-label={m.composer_placeholder()}
          className={cn(
            'max-h-60 resize-none border-0 bg-transparent focus-visible:ring-0 dark:bg-transparent',
            hero ? 'min-h-16' : 'min-h-8 py-1.5'
          )}
        />
        {hero ? <div className='flex justify-end'>{action}</div> : action}
      </form>
      <ComposerFooter />
    </div>
  )
}

function ComposerFooter() {
  const projectTitle = useAppStore(
    (state) =>
      state.projects.find((project) => project.id === state.selectedProjectId)
        ?.title
  )
  const branch = useAppStore(
    (state) =>
      state.threads.find((thread) => thread.id === state.selectedThreadId)
        ?.branch
  )

  return (
    <div className='text-muted-foreground flex items-center gap-3 px-2 text-xs'>
      {projectTitle ? (
        <span className='flex min-w-0 items-center gap-1.5'>
          <HugeiconsIcon
            icon={Folder01Icon}
            strokeWidth={2}
            className='size-3.5 shrink-0'
          />
          <span className='truncate'>{projectTitle}</span>
        </span>
      ) : null}
      <div className='ml-auto flex shrink-0 items-center gap-3'>
        {branch ? (
          <span className='flex items-center gap-1.5'>
            <HugeiconsIcon
              icon={GitBranchIcon}
              strokeWidth={2}
              className='size-3.5'
            />
            {branch}
          </span>
        ) : null}
        <span className='text-foreground font-medium'>{m.claude_code()}</span>
      </div>
    </div>
  )
}
