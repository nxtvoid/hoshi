import { m } from '@/paraglide/messages'
import { getLocale } from '@/paraglide/runtime'
import { formatRelativeTime } from '@hoshi/utils'
import { type ReactNode, useState } from 'react'
import { Button } from '@hoshi/ui/components/button'
import type { ChecksState } from '@/bindings/ChecksState'
import { Textarea } from '@hoshi/ui/components/textarea'
import type { ReviewState } from '@/bindings/ReviewState'
import {
  CancelCircleIcon,
  CheckmarkCircle02Icon,
  Clock01Icon,
  Comment01Icon
} from '@hugeicons/core-free-icons'

export const CHECK_STATE: Record<
  ChecksState,
  { icon: typeof Clock01Icon; color: string }
> = {
  passing: { icon: CheckmarkCircle02Icon, color: 'text-pr-open' },
  failing: { icon: CancelCircleIcon, color: 'text-status-error' },
  pending: { icon: Clock01Icon, color: 'text-status-waiting' }
}

export const REVIEW_STATE: Record<
  ReviewState,
  { icon: typeof Clock01Icon; color: string; label: () => string }
> = {
  requested: {
    icon: Clock01Icon,
    color: 'text-muted-foreground',
    label: m.review_requested
  },
  approved: {
    icon: CheckmarkCircle02Icon,
    color: 'text-pr-open',
    label: m.review_approved
  },
  changesRequested: {
    icon: CancelCircleIcon,
    color: 'text-status-error',
    label: m.review_changes_requested
  },
  commented: {
    icon: Comment01Icon,
    color: 'text-muted-foreground',
    label: m.review_commented
  },
  dismissed: {
    icon: Comment01Icon,
    color: 'text-muted-foreground',
    label: m.review_dismissed
  }
}

export const relativeTime = (iso: string) =>
  formatRelativeTime(iso, new Date(), getLocale())

export function Chip({
  title,
  children
}: {
  title?: string
  children: ReactNode
}) {
  return (
    <span
      title={title}
      className='border-border flex h-6 items-center gap-1 rounded-full border px-2'
    >
      {children}
    </span>
  )
}

export function SectionTitle({ children }: { children: string }) {
  return (
    <h3 className='text-muted-foreground text-[0.6875rem] font-semibold tracking-wide uppercase'>
      {children}
    </h3>
  )
}

/** A text box with Save and Cancel; Ctrl+Enter saves, Escape cancels. */
export function TextEditor({
  initial,
  placeholder,
  saveLabel,
  pending,
  onSave,
  onCancel,
  rows = 4
}: {
  initial: string
  placeholder?: string
  saveLabel: string
  pending: boolean
  /** Resolves to whether it saved; the editor stays open when it did not. */
  onSave: (text: string) => Promise<boolean>
  onCancel?: () => void
  rows?: number
}) {
  const [text, setText] = useState(initial)
  const save = async () => {
    if (await onSave(text)) setText('')
  }

  return (
    <div className='flex flex-col gap-1.5'>
      <Textarea
        autoFocus
        rows={rows}
        value={text}
        placeholder={placeholder}
        aria-label={placeholder ?? saveLabel}
        onChange={(event) => setText(event.currentTarget.value)}
        onKeyDown={(event) => {
          if (event.key === 'Enter' && event.ctrlKey) {
            event.preventDefault()
            void save()
          } else if (event.key === 'Escape' && onCancel) {
            onCancel()
          }
        }}
        className='min-h-0 resize-y text-xs md:text-xs'
      />
      <div className='flex justify-end gap-1.5'>
        {onCancel ? (
          <Button size='xs' variant='ghost' onClick={onCancel}>
            {m.cancel()}
          </Button>
        ) : null}
        <Button
          size='xs'
          disabled={pending || text.trim() === ''}
          onClick={() => void save()}
        >
          {saveLabel}
        </Button>
      </div>
    </div>
  )
}
