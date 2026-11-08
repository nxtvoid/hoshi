import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { InboxIcon } from '@hugeicons/core-free-icons'
import { useSidebarStore } from '@/state/sidebar-store'
import { useInboxBadge } from './use-sidebar-sections'
import { IconButton } from '@hoshi/ui/components/icon-button'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@hoshi/ui/components/tooltip'

function useInbox() {
  const open = useSidebarStore((state) => state.view === 'inbox')
  const setView = useSidebarStore((state) => state.setView)
  const { count, waiting } = useInboxBadge()
  return {
    open,
    count,
    waiting,
    label: count > 0 ? m.inbox_waiting({ count }) : m.inbox(),
    toggle: () => setView(open ? 'project' : 'inbox')
  }
}

function Count({ count, className }: { count: number; className?: string }) {
  if (count === 0) return null
  return (
    <span
      className={cn(
        'bg-foreground text-background flex h-3.5 min-w-3.5 items-center justify-center rounded-full px-1 text-[0.5625rem] leading-none font-semibold tabular-nums',
        className
      )}
    >
      {count > 9 ? '9+' : count}
    </span>
  )
}

/** The inbox at the top of the project rail, above every project. */
export function RailInboxButton() {
  const inbox = useInbox()

  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <button
            type='button'
            aria-label={inbox.label}
            aria-current={inbox.open || undefined}
            onClick={inbox.toggle}
            className={cn(
              'border-sidebar-border text-sidebar-foreground/80 hover:bg-sidebar-accent focus-visible:ring-sidebar-ring relative flex size-8 shrink-0 items-center justify-center rounded-lg border outline-none focus-visible:ring-2',
              inbox.open &&
                'border-sidebar-foreground bg-sidebar-accent text-sidebar-accent-foreground',
              inbox.count > 0 && [
                'motion-safe:animate-signal',
                inbox.waiting
                  ? 'border-status-waiting [--signal:var(--status-waiting)]'
                  : 'border-status-done [--signal:var(--status-done)]'
              ]
            )}
          />
        }
      >
        <HugeiconsIcon icon={InboxIcon} strokeWidth={2} className='size-4' />
        <Count count={inbox.count} className='absolute -top-1.5 -right-1.5' />
      </TooltipTrigger>
      <TooltipContent side='right' sideOffset={12}>
        {inbox.label}
      </TooltipContent>
    </Tooltip>
  )
}

/** The inbox in the sidebar footer, while there is no rail to hold it. */
export function FooterInboxButton() {
  const inbox = useInbox()

  return (
    <IconButton
      label={inbox.label}
      side='top'
      active={inbox.open}
      onClick={inbox.toggle}
      className='relative'
    >
      <HugeiconsIcon icon={InboxIcon} strokeWidth={2} />
      <Count count={inbox.count} className='absolute -top-0.5 -right-0.5' />
    </IconButton>
  )
}
