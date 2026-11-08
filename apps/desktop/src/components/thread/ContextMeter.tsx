import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { getLocale } from '@/paraglide/runtime'
import { useAppStore } from '@/state/app-store'
import { contextPercent, formatTokens } from '@/lib/usage'
import {
  HoverCard,
  HoverCardContent,
  HoverCardTrigger
} from '@hoshi/ui/components/hover-card'

const RADIUS = 9
const CIRCUMFERENCE = 2 * Math.PI * RADIUS
const CROWDED_PERCENT = 90

export function ContextMeter() {
  const usage = useAppStore((state) =>
    state.selectedThreadId
      ? state.contextUsages[state.selectedThreadId]
      : undefined
  )
  if (!usage) return null

  const locale = getLocale()
  const percent = contextPercent(usage)
  const used = formatTokens(usage.usedTokens, locale)
  const shown = percent === null ? null : Math.round(percent)
  const crowded = percent !== null && percent > CROWDED_PERCENT
  const label =
    shown === null
      ? m.context_tokens_used({ used })
      : m.context_window_usage({
          percent: shown,
          used,
          max: formatTokens(usage.maxTokens ?? 0, locale)
        })

  return (
    <HoverCard>
      <HoverCardTrigger
        delay={150}
        aria-label={`${m.context_window()}: ${label}`}
        className={cn(
          'hover:text-foreground flex h-6 cursor-default items-center gap-1.5 rounded-md px-1.5 tabular-nums transition-colors',
          crowded && 'text-status-error hover:text-status-error'
        )}
      >
        <Ring percent={percent ?? 0} />
        <span>{shown === null ? used : `${shown}%`}</span>
      </HoverCardTrigger>
      <HoverCardContent
        side='top'
        align='end'
        className='flex w-60 flex-col gap-2 text-xs'
      >
        <div className='flex items-center justify-between gap-3'>
          <span className='text-muted-foreground'>{m.context_window()}</span>
          <span className='font-medium tabular-nums'>{label}</span>
        </div>
        {percent === null ? null : (
          <div className='bg-muted h-1.5 overflow-hidden rounded-full'>
            <div
              className={cn(
                'h-full rounded-full transition-[width] duration-500',
                crowded ? 'bg-status-error' : 'bg-muted-foreground'
              )}
              style={{ width: `${percent}%` }}
            />
          </div>
        )}
        <p className='text-muted-foreground'>
          {percent === null
            ? m.context_window_size_pending()
            : m.context_compacts_automatically()}
        </p>
      </HoverCardContent>
    </HoverCard>
  )
}

function Ring({ percent }: { percent: number }) {
  return (
    <svg viewBox='0 0 24 24' aria-hidden='true' className='size-3.5 -rotate-90'>
      <circle
        cx='12'
        cy='12'
        r={RADIUS}
        fill='none'
        stroke='currentColor'
        strokeWidth='3.5'
        className='opacity-25'
      />
      <circle
        cx='12'
        cy='12'
        r={RADIUS}
        fill='none'
        stroke='currentColor'
        strokeWidth='3.5'
        strokeLinecap='round'
        strokeDasharray={CIRCUMFERENCE}
        strokeDashoffset={CIRCUMFERENCE * (1 - percent / 100)}
        className='transition-[stroke-dashoffset] duration-500 motion-reduce:transition-none'
      />
    </svg>
  )
}
