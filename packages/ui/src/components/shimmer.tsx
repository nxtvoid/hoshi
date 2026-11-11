import { cn } from 'cn'
import type { ReactNode } from 'react'

/**
 * Its content with a highlight sweeping across it, for what is happening right now. The
 * highlight is a copy of the content in the foreground color, seen through a moving window.
 */
export function Shimmer({
  children,
  className
}: {
  children: ReactNode
  /** Lays out the content, the same for the copy the highlight shows. */
  className?: string
}) {
  return (
    <span className='relative flex min-w-0 overflow-hidden'>
      <span className={cn('flex min-w-0', className)}>{children}</span>
      <span
        aria-hidden
        className='shimmer-sweep pointer-events-none absolute inset-y-0 select-none'
      >
        <span className='shimmer-sweep-counter block'>
          <span
            className={cn(
              'shimmer-sweep-aligned text-foreground flex min-w-0',
              className
            )}
          >
            {children}
          </span>
        </span>
      </span>
    </span>
  )
}
