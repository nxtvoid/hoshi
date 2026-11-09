import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { SIGNAL_BORDER } from './signal-border'
import { useAppStore } from '@/state/app-store'
import { HugeiconsIcon } from '@hugeicons/react'
import { useUnseenStore } from '@/state/unseen-store'
import { useSidebarStore } from '@/state/sidebar-store'
import { projectSignal } from '@/state/thread-signals'
import { BubbleChatIcon } from '@hugeicons/core-free-icons'
import { IconButton } from '@hoshi/ui/components/icon-button'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@hoshi/ui/components/tooltip'

function useChats() {
  const open = useSidebarStore((state) => state.view === 'chats')
  const setView = useSidebarStore((state) => state.setView)
  const unseen = useUnseenStore((state) => state.unseen)
  const signal = useAppStore((state) =>
    projectSignal(state.threads, null, unseen)
  )
  return {
    open,
    signal,
    toggle: () => setView(open ? 'project' : 'chats'),
    show: () => setView('chats')
  }
}

/** The chats, below the inbox on the project rail. */
export function RailChatsButton() {
  const chats = useChats()

  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <button
            type='button'
            aria-label={m.chats()}
            aria-current={chats.open || undefined}
            onClick={chats.toggle}
            className={cn(
              'border-sidebar-border text-sidebar-foreground/80 hover:bg-sidebar-accent focus-visible:ring-sidebar-ring relative flex size-8 shrink-0 items-center justify-center rounded-lg border outline-none focus-visible:ring-2',
              chats.open &&
                'border-sidebar-foreground bg-sidebar-accent text-sidebar-accent-foreground',
              chats.signal && [
                SIGNAL_BORDER[chats.signal],
                'motion-safe:animate-signal'
              ]
            )}
          />
        }
      >
        <HugeiconsIcon
          icon={BubbleChatIcon}
          strokeWidth={2}
          className='size-4'
        />
      </TooltipTrigger>
      <TooltipContent side='right' sideOffset={12}>
        {m.chats()}
      </TooltipContent>
    </Tooltip>
  )
}

/** The chats in the sidebar footer, while there is no rail to hold them. */
export function FooterChatsButton() {
  const chats = useChats()

  return (
    <IconButton
      label={m.chats()}
      side='top'
      active={chats.open}
      onClick={chats.show}
    >
      <HugeiconsIcon icon={BubbleChatIcon} strokeWidth={2} />
    </IconButton>
  )
}
