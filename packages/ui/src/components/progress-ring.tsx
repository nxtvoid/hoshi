import type * as React from 'react'

import { cn } from 'cn'

const RADIUS = 6
const CIRCUMFERENCE = 2 * Math.PI * RADIUS

/** A determinate ring; `value` is 0–100, or `null` while unknown, which spins a quarter arc. */
function ProgressRing({
  value,
  className,
  ...props
}: React.ComponentProps<'svg'> & { value: number | null }) {
  const progress = value === null ? 25 : Math.min(100, Math.max(0, value))

  return (
    <svg
      data-slot='progress-ring'
      viewBox='0 0 16 16'
      fill='none'
      aria-hidden
      className={cn(
        'size-4 -rotate-90',
        value === null && 'animate-spin',
        className
      )}
      {...props}
    >
      <circle
        cx='8'
        cy='8'
        r={RADIUS}
        strokeWidth='2'
        className='stroke-current opacity-20'
      />
      <circle
        cx='8'
        cy='8'
        r={RADIUS}
        strokeWidth='2'
        strokeLinecap='round'
        strokeDasharray={CIRCUMFERENCE}
        strokeDashoffset={CIRCUMFERENCE * (1 - progress / 100)}
        className='stroke-current transition-[stroke-dashoffset]'
      />
    </svg>
  )
}

export { ProgressRing }
