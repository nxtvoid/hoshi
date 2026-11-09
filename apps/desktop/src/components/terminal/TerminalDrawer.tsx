import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { LazyTerminalView } from './LazyTerminalView'
import { IconButton } from '@hoshi/ui/components/icon-button'
import { ResizeHandle } from '@hoshi/ui/components/resize-handle'
import {
  Add01Icon,
  ArrowDown01Icon,
  Cancel01Icon,
  ComputerTerminal01Icon
} from '@hugeicons/core-free-icons'
import {
  DRAWER_HEIGHT,
  useTerminalDrawer,
  useTerminalStore
} from '@/state/terminal-store'

/** The terminals docked below a thread's chat. */
export function TerminalDrawer({ threadId }: { threadId: string }) {
  const { open, terminalIds, activeId } = useTerminalDrawer(threadId)
  const height = useTerminalStore((state) => state.height)
  const setHeight = useTerminalStore((state) => state.setHeight)
  const toggleDrawer = useTerminalStore((state) => state.toggleDrawer)
  const addTerminal = useTerminalStore((state) => state.addTerminal)
  if (!open) return null

  return (
    <section
      aria-label={m.surface_terminal()}
      style={{ height }}
      className='border-border bg-background relative flex max-h-[70%] shrink-0 flex-col border-t'
    >
      <ResizeHandle
        edge='top'
        label={m.resize_terminal()}
        value={height}
        min={DRAWER_HEIGHT.min}
        max={DRAWER_HEIGHT.max}
        defaultValue={DRAWER_HEIGHT.default}
        onChange={setHeight}
      />
      <div className='flex h-8 shrink-0 items-center gap-1 pr-1 pl-2'>
        <div
          role='tablist'
          className='flex min-w-0 flex-1 items-center gap-1 overflow-x-auto'
        >
          {terminalIds.map((terminalId, index) => (
            <DrawerTab
              key={terminalId}
              threadId={threadId}
              terminalId={terminalId}
              number={index + 1}
              active={terminalId === activeId}
            />
          ))}
        </div>
        <IconButton
          label={m.new_terminal()}
          onClick={() => addTerminal(threadId)}
        >
          <HugeiconsIcon icon={Add01Icon} strokeWidth={2} />
        </IconButton>
        <IconButton
          label={m.hide_terminal()}
          onClick={() => toggleDrawer(threadId)}
        >
          <HugeiconsIcon icon={ArrowDown01Icon} strokeWidth={2} />
        </IconButton>
      </div>
      {terminalIds.map((terminalId) => (
        <div
          key={terminalId}
          role='tabpanel'
          hidden={terminalId !== activeId}
          className='flex min-h-0 flex-1 flex-col'
        >
          <LazyTerminalView
            threadId={threadId}
            terminalId={terminalId}
            onOpenHere={() => addTerminal(threadId)}
          />
        </div>
      ))}
    </section>
  )
}

function DrawerTab({
  threadId,
  terminalId,
  number,
  active
}: {
  threadId: string
  terminalId: string
  number: number
  active: boolean
}) {
  const activate = useTerminalStore((state) => state.activateTerminal)
  const close = useTerminalStore((state) => state.closeTerminal)
  const label = m.terminal_tab({ number })

  return (
    <div
      className={cn(
        'group flex h-6 shrink-0 items-center rounded-md text-xs',
        active
          ? 'bg-secondary text-secondary-foreground'
          : 'text-muted-foreground hover:bg-accent/60'
      )}
    >
      <button
        type='button'
        role='tab'
        aria-selected={active}
        onClick={() => activate(threadId, terminalId)}
        onAuxClick={(event) => {
          if (event.button === 1) close(threadId, terminalId)
        }}
        className='focus-visible:ring-ring flex h-full items-center gap-1.5 rounded-md pl-2 outline-none focus-visible:ring-2'
      >
        <HugeiconsIcon
          icon={ComputerTerminal01Icon}
          strokeWidth={2}
          className='size-3.5'
        />
        {label}
      </button>
      <button
        type='button'
        aria-label={m.close_surface({ surface: label })}
        onClick={() => close(threadId, terminalId)}
        className='hover:bg-accent focus-visible:ring-ring mx-0.5 flex size-4 items-center justify-center rounded-sm opacity-60 outline-none group-hover:opacity-100 focus-visible:opacity-100 focus-visible:ring-2'
      >
        <HugeiconsIcon icon={Cancel01Icon} strokeWidth={2} className='size-3' />
      </button>
    </div>
  )
}
