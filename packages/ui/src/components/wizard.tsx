import { cn } from 'cn'
import type { ReactNode, RefObject } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import { Button } from '@hoshi/ui/components/button'
import { Dialog as DialogPrimitive } from '@base-ui/react/dialog'
import { Cancel01Icon, Tick02Icon } from '@hugeicons/core-free-icons'

type WizardProps = {
  open: boolean
  /** Called on Esc and the close button; a press on the backdrop does nothing. */
  onOpenChange: (open: boolean) => void
  /** Called after the close animation, so the caller can leave the route. */
  onExited?: () => void
  label: string
  /** Gets the focus when the wizard opens; the first focusable element otherwise. */
  initialFocus?: RefObject<HTMLElement | null>
  /** Rendered over the backdrop's top edge, such as a window drag strip. */
  topBar?: ReactNode
  children: ReactNode
}

/** A centered dialog that walks through a few steps. */
export function Wizard({
  open,
  onOpenChange,
  onExited,
  label,
  initialFocus,
  topBar,
  children
}: WizardProps) {
  return (
    <DialogPrimitive.Root
      open={open}
      disablePointerDismissal
      onOpenChange={(nextOpen) => onOpenChange(nextOpen)}
      onOpenChangeComplete={(nextOpen) => {
        if (!nextOpen) onExited?.()
      }}
    >
      <DialogPrimitive.Portal>
        <DialogPrimitive.Backdrop className='bg-backdrop fixed inset-0 z-50 transition-opacity duration-200 data-ending-style:opacity-0 data-starting-style:opacity-0 supports-backdrop-filter:backdrop-blur-sm' />
        <DialogPrimitive.Viewport className='fixed inset-0 z-50 flex items-center justify-center p-6 pt-12'>
          {topBar ? (
            <div className='absolute inset-x-0 top-0'>{topBar}</div>
          ) : null}
          <DialogPrimitive.Popup
            aria-label={label}
            initialFocus={initialFocus}
            finalFocus={false}
            className='bg-popover text-popover-foreground relative flex max-h-full w-full max-w-3xl min-w-0 flex-col overflow-hidden rounded-2xl border shadow-2xl transition-[opacity,scale] duration-200 ease-out outline-none data-ending-style:scale-[0.985] data-ending-style:opacity-0 data-starting-style:scale-[0.985] data-starting-style:opacity-0'
          >
            {children}
          </DialogPrimitive.Popup>
        </DialogPrimitive.Viewport>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  )
}

/** The brand on the left, the close button on the right, and the steps below. */
export function WizardHeader({
  brand,
  closeLabel,
  children
}: {
  brand: ReactNode
  closeLabel: string
  children?: ReactNode
}) {
  return (
    <div className='flex shrink-0 flex-col gap-4 px-6 pt-5 pb-5'>
      <div className='flex items-center justify-between gap-3'>
        {brand}
        <DialogPrimitive.Close
          aria-label={closeLabel}
          render={<Button variant='ghost' size='icon-sm' className='-me-2' />}
        >
          <HugeiconsIcon icon={Cancel01Icon} strokeWidth={2} />
        </DialogPrimitive.Close>
      </div>
      {children}
    </div>
  )
}

/** A segmented track of steps; only the steps before `current` can be picked. */
export function WizardSteps({
  label,
  steps,
  current,
  onSelect
}: {
  label: string
  steps: readonly string[]
  current: number
  onSelect: (step: number) => void
}) {
  return (
    <ol
      aria-label={label}
      className='bg-muted/60 grid auto-cols-fr grid-flow-col gap-1 rounded-xl p-1'
    >
      {steps.map((step, index) => {
        const done = index < current
        const active = index === current
        return (
          <li key={step} className='min-w-0'>
            <button
              type='button'
              disabled={!done}
              aria-current={active ? 'step' : undefined}
              onClick={() => onSelect(index)}
              className={cn(
                'focus-visible:ring-ring flex w-full min-w-0 items-center gap-2 rounded-lg px-2.5 py-1.5 text-left transition-colors outline-none focus-visible:ring-2 focus-visible:ring-inset',
                done && 'hover:bg-background/60 cursor-pointer',
                active && 'bg-background ring-border shadow-xs ring-1'
              )}
            >
              <span
                aria-hidden='true'
                className={cn(
                  'grid size-5 shrink-0 place-items-center rounded-full text-xs font-medium tabular-nums',
                  done && 'bg-primary text-primary-foreground',
                  active &&
                    'bg-primary/10 text-foreground ring-primary/25 ring-1',
                  !done &&
                    !active &&
                    'bg-background text-muted-foreground ring-border ring-1'
                )}
              >
                {done ? (
                  <HugeiconsIcon
                    icon={Tick02Icon}
                    strokeWidth={2.5}
                    className='size-3'
                  />
                ) : (
                  index + 1
                )}
              </span>
              <span
                className={cn(
                  'min-w-0 truncate text-sm font-medium',
                  active ? 'text-foreground' : 'text-muted-foreground'
                )}
              >
                {step}
              </span>
            </button>
          </li>
        )
      })}
    </ol>
  )
}

/** The inset band that holds the current step. */
export function WizardPanel({
  className,
  children
}: {
  className?: string
  children: ReactNode
}) {
  return (
    <div
      className={cn(
        'bg-muted/40 min-h-0 flex-1 overflow-y-auto border-y px-8 py-7',
        className
      )}
    >
      {children}
    </div>
  )
}

export function WizardTitle({ children }: { children: ReactNode }) {
  return (
    <h2 className='text-foreground text-2xl font-semibold tracking-tight'>
      {children}
    </h2>
  )
}

export function WizardDescription({ children }: { children: ReactNode }) {
  return (
    <p className='text-muted-foreground mt-2 text-sm leading-relaxed'>
      {children}
    </p>
  )
}

/** `leading` sits on the left, such as a skip button; the main actions on the right. */
export function WizardFooter({
  leading,
  children
}: {
  leading?: ReactNode
  children: ReactNode
}) {
  return (
    <div className='flex shrink-0 items-center gap-2 px-6 py-4'>
      {leading}
      <div className='ms-auto flex items-center gap-2'>{children}</div>
    </div>
  )
}
