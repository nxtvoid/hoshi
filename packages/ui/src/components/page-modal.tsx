import { HugeiconsIcon } from '@hugeicons/react'
import { useRef, useState, type ReactNode } from 'react'
import { Button } from '@hoshi/ui/components/button'
import { Cancel01Icon } from '@hugeicons/core-free-icons'
import { Dialog as DialogPrimitive } from '@base-ui/react/dialog'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@hoshi/ui/components/tooltip'

type PageModalProps = {
  label: string
  /** Called after the close animation, so the caller can leave the route. */
  onClose: () => void
  /** Rendered over the backdrop's top edge, such as a window drag strip. */
  topBar?: ReactNode
  children: ReactNode
}

/**
 * A large modal for a whole page, such as Settings. Esc, the close button or a
 * press on the backdrop closes it.
 */
export function PageModal({
  label,
  onClose,
  topBar,
  children
}: PageModalProps) {
  const [open, setOpen] = useState(true)
  const popupRef = useRef<HTMLDivElement>(null)
  const backdropPress = useRef(false)

  return (
    <DialogPrimitive.Root
      open={open}
      disablePointerDismissal
      onOpenChange={(nextOpen) => {
        if (nextOpen) return
        // Closing unmounts a focused field without a blur, which would drop a commit-on-blur edit.
        const focused = document.activeElement
        if (
          focused instanceof HTMLElement &&
          popupRef.current?.contains(focused)
        ) {
          focused.blur()
        }
        setOpen(false)
      }}
      onOpenChangeComplete={(nextOpen) => {
        if (!nextOpen) onClose()
      }}
    >
      <DialogPrimitive.Portal>
        <DialogPrimitive.Backdrop className='bg-backdrop fixed inset-0 z-50 transition-opacity duration-150 data-ending-style:opacity-0 data-starting-style:opacity-0 supports-backdrop-filter:backdrop-blur-xs' />
        <DialogPrimitive.Viewport
          className='fixed inset-0 z-50 flex items-center justify-center p-6 pt-12 lg:p-10 lg:pt-12'
          onPointerDown={(event) => {
            backdropPress.current = event.target === event.currentTarget
          }}
          onClick={(event) => {
            if (backdropPress.current && event.target === event.currentTarget) {
              setOpen(false)
            }
            backdropPress.current = false
          }}
        >
          {topBar ? (
            <div className='absolute inset-x-0 top-0'>{topBar}</div>
          ) : null}
          <DialogPrimitive.Popup
            ref={popupRef}
            initialFocus={popupRef}
            finalFocus={false}
            aria-label={label}
            className='bg-background text-foreground relative flex size-full max-h-240 min-h-0 max-w-320 min-w-0 flex-col overflow-hidden rounded-2xl border shadow-2xl transition-[opacity,scale] duration-150 ease-out outline-none data-ending-style:scale-[0.985] data-ending-style:opacity-0 data-starting-style:scale-[0.985] data-starting-style:opacity-0'
          >
            {children}
          </DialogPrimitive.Popup>
        </DialogPrimitive.Viewport>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  )
}

export function PageModalClose({ label }: { label: string }) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <DialogPrimitive.Close
            aria-label={label}
            render={<Button variant='ghost' size='icon-sm' />}
          />
        }
      >
        <HugeiconsIcon icon={Cancel01Icon} strokeWidth={2} />
      </TooltipTrigger>
      <TooltipContent side='bottom'>{label}</TooltipContent>
    </Tooltip>
  )
}
