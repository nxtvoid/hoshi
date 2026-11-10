import { useEffect, useRef, useState } from 'react'
import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { showNotes } from '@/state/notes-hub'
import { useNotesStore } from '@/state/notes-store'
import { useSidebarStore } from '@/state/sidebar-store'
import { Note01Icon, Tick02Icon } from '@hugeicons/core-free-icons'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@hoshi/ui/components/tooltip'

const CAPTURED_MS = 1500

/** Whether a note was just kept from elsewhere in the app. */
function useJustCaptured() {
  const captures = useNotesStore((state) => state.captures)
  const seen = useRef(captures)
  const [captured, setCaptured] = useState(false)

  useEffect(() => {
    if (captures === seen.current) return
    seen.current = captures
    setCaptured(true)
    const timer = setTimeout(() => setCaptured(false), CAPTURED_MS)
    return () => clearTimeout(timer)
  }, [captures])

  return captured
}

/** The user's notes, below the chats on the project rail. */
export function RailNotesButton() {
  const open = useSidebarStore((state) => state.notesOpen)
  const setNotesOpen = useSidebarStore((state) => state.setNotesOpen)
  const captured = useJustCaptured()
  const label = captured ? m.notes_saved() : m.notes()

  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <button
            type='button'
            aria-label={label}
            aria-current={open || undefined}
            onClick={() => (open ? setNotesOpen(false) : showNotes())}
            className={cn(
              'border-sidebar-border text-sidebar-foreground/80 hover:bg-sidebar-accent focus-visible:ring-sidebar-ring relative flex size-8 shrink-0 items-center justify-center rounded-lg border outline-none focus-visible:ring-2',
              open &&
                'border-sidebar-foreground bg-sidebar-accent text-sidebar-accent-foreground',
              captured && 'border-status-done text-status-done'
            )}
          />
        }
      >
        <HugeiconsIcon
          icon={captured ? Tick02Icon : Note01Icon}
          strokeWidth={2}
          className='size-4'
        />
      </TooltipTrigger>
      <TooltipContent side='right' sideOffset={12}>
        {label}
      </TooltipContent>
    </Tooltip>
  )
}
