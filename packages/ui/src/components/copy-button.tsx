import { cn } from 'cn'
import { Tick02Icon } from '@hugeicons/core-free-icons'
import { type ReactNode, useEffect, useRef, useState } from 'react'
import { HugeiconsIcon, type IconSvgElement } from '@hugeicons/react'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@hoshi/ui/components/tooltip'

const COPIED_MS = 1500

const ICON =
  'col-start-1 row-start-1 size-full transition-[opacity,scale,filter] duration-200 ease-out motion-reduce:transition-none'
const SHOWN = 'scale-100 opacity-100 blur-none'
const HIDDEN = 'scale-50 opacity-0 blur-[2px]'

type CopyButtonProps = {
  value: string
  /** Tooltip and accessible name before copying. */
  label: string
  /** Tooltip and accessible name for a moment after copying. */
  copiedLabel: string
  /** The icon at rest; it turns into a check after copying. */
  icon: IconSvgElement
  iconClassName?: string
  side?: 'top' | 'bottom' | 'left' | 'right'
  className?: string
  /** Shown after the icon, such as the value itself. */
  children?: ReactNode
}

/** Copies `value` on click, with its label in a tooltip and the icon easing into a check. */
export function CopyButton({
  value,
  label,
  copiedLabel,
  icon,
  iconClassName,
  side = 'bottom',
  className,
  children
}: CopyButtonProps) {
  const [copied, setCopied] = useState(false)
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current)
    },
    []
  )

  const copy = () => {
    void navigator.clipboard.writeText(value).then(() => {
      if (timer.current) clearTimeout(timer.current)
      setCopied(true)
      timer.current = setTimeout(() => setCopied(false), COPIED_MS)
    })
  }

  const text = copied ? copiedLabel : label

  return (
    <Tooltip>
      <TooltipTrigger
        closeOnClick={false}
        render={
          <button
            type='button'
            aria-label={text}
            onClick={copy}
            className={cn(
              'focus-visible:ring-ring inline-flex cursor-pointer items-center gap-1 outline-none focus-visible:ring-2',
              className
            )}
          />
        }
      >
        <span
          aria-hidden='true'
          className={cn('grid size-3.5 shrink-0', iconClassName)}
        >
          <HugeiconsIcon
            icon={icon}
            strokeWidth={2}
            className={cn(ICON, copied ? HIDDEN : SHOWN)}
          />
          <HugeiconsIcon
            icon={Tick02Icon}
            strokeWidth={2}
            className={cn(ICON, copied ? SHOWN : HIDDEN)}
          />
        </span>
        {children}
      </TooltipTrigger>
      <TooltipContent side={side}>{text}</TooltipContent>
    </Tooltip>
  )
}
