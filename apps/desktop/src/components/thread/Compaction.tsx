import { create } from 'zustand'
import { cn } from '@hoshi/utils'
import { useEffect, useState } from 'react'
import { m } from '@/paraglide/messages'
import { formatTokens } from '@/lib/usage'
import { getLocale } from '@/paraglide/runtime'
import { HugeiconsIcon } from '@hugeicons/react'
import { compactionDue } from '@/lib/compaction'
import { useAppStore } from '@/state/app-store'
import type { ItemContent } from '@/bindings/ItemContent'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@hoshi/ui/components/tooltip'
import { ArrowShrink01Icon, Loading03Icon } from '@hugeicons/core-free-icons'

/** How often a stale cache is looked for while the composer shows. */
const CHECK_EVERY_MS = 30_000

/** Threads whose user chose to keep their full history on the next send. */
const useKeptStore = create<{
  kept: Record<string, boolean>
  toggle: (threadId: string) => void
}>()((set) => ({
  kept: {},
  toggle: (threadId) =>
    set((state) => ({
      kept: { ...state.kept, [threadId]: !state.kept[threadId] }
    }))
}))

/**
 * Whether the selected thread's next send compacts first, as t3code offers: its context
 * is large and its prompt cache ran out. `tokens` is its size; `compact` is the user's
 * choice for this send.
 */
export function useCompactionDue(): {
  tokens: number
  compact: boolean
} | null {
  const threadId = useAppStore((state) => state.selectedThreadId)
  const usage = useAppStore((state) =>
    state.selectedThreadId
      ? state.contextUsages[state.selectedThreadId]
      : undefined
  )
  const working = useAppStore(
    (state) =>
      state.threads.find((thread) => thread.id === state.selectedThreadId)
        ?.activeTurnId != null
  )
  const kept = useKeptStore((state) =>
    threadId ? state.kept[threadId] === true : false
  )
  const [now, setNow] = useState(Date.now())
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), CHECK_EVERY_MS)
    return () => clearInterval(timer)
  }, [])
  if (!threadId || !usage || !compactionDue(usage, now, working)) return null
  return { tokens: usage.usedTokens, compact: !kept }
}

/** Chooses between compacting before the next send and keeping all of the history. */
export function CompactionChip() {
  const due = useCompactionDue()
  const threadId = useAppStore((state) => state.selectedThreadId)
  const toggle = useKeptStore((state) => state.toggle)
  if (!due || !threadId) return null
  const tokens = formatTokens(due.tokens, getLocale())
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <button
            type='button'
            aria-pressed={due.compact}
            onClick={() => toggle(threadId)}
            className={cn(
              'hover:text-foreground flex h-6 items-center gap-1 rounded-md px-1.5 tabular-nums transition-colors',
              due.compact && 'text-foreground'
            )}
          />
        }
      >
        <HugeiconsIcon
          icon={ArrowShrink01Icon}
          strokeWidth={2}
          className='size-3.5'
        />
        {due.compact
          ? m.compaction_chip_compact({ tokens })
          : m.compaction_chip_keep({ tokens })}
      </TooltipTrigger>
      <TooltipContent className='max-w-64'>
        {due.compact
          ? m.compaction_chip_compact_hint({ tokens })
          : m.compaction_chip_keep_hint({ tokens })}
      </TooltipContent>
    </Tooltip>
  )
}

/** The conversation compacting, or compacted, as a divider in the transcript. */
export function CompactionRow({
  content
}: {
  content: Extract<ItemContent, { kind: 'compaction' }>
}) {
  const locale = getLocale()
  const label =
    content.status === 'running'
      ? m.compaction_running()
      : content.status === 'failed'
        ? m.compaction_failed()
        : content.beforeTokens !== null && content.afterTokens !== null
          ? m.compaction_done_sizes({
              before: formatTokens(content.beforeTokens, locale),
              after: formatTokens(content.afterTokens, locale)
            })
          : m.compaction_done()
  return (
    <div
      className={cn(
        'text-muted-foreground flex items-center gap-3 text-xs',
        content.status === 'failed' && 'text-status-error'
      )}
    >
      <span className='bg-border h-px flex-1' />
      <span className='flex min-w-0 items-center gap-1.5'>
        <HugeiconsIcon
          icon={
            content.status === 'running' ? Loading03Icon : ArrowShrink01Icon
          }
          strokeWidth={2}
          className={cn(
            'size-3.5 shrink-0',
            content.status === 'running' && 'animate-spin'
          )}
        />
        <span className='truncate'>
          {label}
          {content.error ? ` · ${content.error}` : null}
        </span>
      </span>
      <span className='bg-border h-px flex-1' />
    </div>
  )
}
