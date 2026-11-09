import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { useDialogStore } from '@/state/dialog-store'
import type { DialogTone } from '@/state/dialog-store'
import {
  Alert02Icon,
  CheckmarkCircle02Icon,
  Delete02Icon,
  HelpCircleIcon
} from '@hugeicons/core-free-icons'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogTitle
} from '@hoshi/ui/components/alert-dialog'

/** Each tone's icon, its haloed badge, and the color of the button that goes ahead. */
const TONE: Record<
  DialogTone,
  { icon: typeof HelpCircleIcon; badge: string; action: string }
> = {
  default: {
    icon: HelpCircleIcon,
    badge: 'bg-status-working/12 text-status-working ring-status-working/6',
    action: ''
  },
  destructive: {
    icon: Delete02Icon,
    badge: 'bg-destructive/12 text-destructive ring-destructive/6',
    action:
      'bg-destructive text-destructive-foreground hover:bg-destructive/90 focus-visible:ring-destructive/40'
  },
  warning: {
    icon: Alert02Icon,
    badge: 'bg-status-waiting/12 text-status-waiting ring-status-waiting/6',
    action: ''
  },
  success: {
    icon: CheckmarkCircle02Icon,
    badge: 'bg-status-done/12 text-status-done ring-status-done/6',
    action: ''
  }
}

const BUTTON = 'h-9 rounded-lg px-4'

/** Shows what `confirm` and `notify` ask, one at a time. Mounted once, in the app. */
export function DialogHost() {
  const current = useDialogStore((state) => state.current)
  const open = useDialogStore((state) => state.open)
  const settle = useDialogStore((state) => state.settle)
  const tone = TONE[current?.tone ?? 'default']

  return (
    <AlertDialog
      open={open}
      onOpenChange={(next) => {
        if (!next) settle(null)
      }}
    >
      <AlertDialogContent className='flex flex-col gap-0 rounded-2xl p-5 shadow-2xl data-[size=default]:max-w-[calc(100%-2rem)] data-[size=default]:sm:max-w-100'>
        {current ? (
          <>
            <span
              className={cn(
                'flex size-11 shrink-0 items-center justify-center rounded-full ring-[6px]',
                tone.badge
              )}
            >
              <HugeiconsIcon
                icon={current.icon ?? tone.icon}
                strokeWidth={2}
                className='size-5'
              />
            </span>
            <AlertDialogTitle className='mt-4 text-base leading-snug font-semibold'>
              {current.title}
            </AlertDialogTitle>
            {current.description ? (
              <AlertDialogDescription className='mt-1.5 text-pretty'>
                {current.description}
              </AlertDialogDescription>
            ) : null}
            <div className='mt-6 flex justify-end gap-2'>
              {current.cancelLabel === false ? null : (
                <AlertDialogCancel className={BUTTON}>
                  {current.cancelLabel ?? m.cancel()}
                </AlertDialogCancel>
              )}
              {current.alternativeLabel ? (
                <AlertDialogAction
                  variant='outline'
                  className={BUTTON}
                  onClick={() => settle('alternative')}
                >
                  {current.alternativeLabel}
                </AlertDialogAction>
              ) : null}
              <AlertDialogAction
                autoFocus
                className={cn(BUTTON, tone.action)}
                onClick={() => settle('confirm')}
              >
                {current.confirmLabel}
              </AlertDialogAction>
            </div>
          </>
        ) : null}
      </AlertDialogContent>
    </AlertDialog>
  )
}
