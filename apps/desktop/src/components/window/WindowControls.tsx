import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { type ReactNode, useEffect, useState } from 'react'
import {
  Cancel01Icon,
  Copy01Icon,
  MinusSignIcon,
  SquareIcon
} from '@hugeicons/core-free-icons'

const appWindow = getCurrentWindow()

export function WindowControls() {
  const [maximized, setMaximized] = useState(false)

  // The OS can maximize the window (snap, double-click), so follow its resize events.
  useEffect(() => {
    let active = true
    const sync = () =>
      appWindow.isMaximized().then((value) => active && setMaximized(value))
    sync()
    const unlisten = appWindow.onResized(sync)
    return () => {
      active = false
      unlisten.then((stop) => stop())
    }
  }, [])

  return (
    // Above page modals (z-50), so the window can always be minimized or closed.
    <div className='relative z-60 flex h-full shrink-0'>
      <ControlButton
        label={m.window_minimize()}
        onClick={() => appWindow.minimize()}
      >
        <HugeiconsIcon icon={MinusSignIcon} />
      </ControlButton>
      <ControlButton
        label={maximized ? m.window_restore() : m.window_maximize()}
        onClick={() => appWindow.toggleMaximize()}
      >
        {maximized ? (
          <HugeiconsIcon icon={Copy01Icon} className='-scale-x-100' />
        ) : (
          <HugeiconsIcon icon={SquareIcon} className='size-3.5' />
        )}
      </ControlButton>
      <ControlButton
        label={m.window_close()}
        onClick={() => appWindow.close()}
        className='hover:bg-window-close hover:text-window-close-foreground'
      >
        <HugeiconsIcon icon={Cancel01Icon} />
      </ControlButton>
    </div>
  )
}

type ControlButtonProps = {
  label: string
  onClick: () => void
  className?: string
  children: ReactNode
}

function ControlButton({
  label,
  onClick,
  className,
  children
}: ControlButtonProps) {
  return (
    <button
      type='button'
      aria-label={label}
      title={label}
      onClick={onClick}
      className={cn(
        'text-foreground hover:bg-accent focus-visible:bg-accent flex h-full w-11 items-center justify-center outline-none [&_svg:not([class*="size-"])]:size-4',
        className
      )}
    >
      {children}
    </button>
  )
}
