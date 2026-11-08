import { useState } from 'react'
import { m } from '@/paraglide/messages'
import type { Thread } from '@/bindings/Thread'
import { useAppStore } from '@/state/app-store'
import { HugeiconsIcon } from '@hugeicons/react'
import { CleanIcon } from '@hugeicons/core-free-icons'
import { confirm, notify } from '@/state/dialog-store'
import { mergedCleanup } from '@/state/merged-cleanup'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@hoshi/ui/components/tooltip'
import {
  cleanupErrorMessage,
  runCleanup
} from '@/components/thread/use-workspace-cleanup'

/**
 * Cleans up every merged thread of a section that still has a worktree, one after another.
 * Each one is checked on its own; those that would lose work are kept and listed after.
 */
export function CleanUpMergedButton({ threads }: { threads: Thread[] }) {
  const pullRequests = useAppStore((state) => state.pullRequests)
  const [running, setRunning] = useState(false)
  const merged = threads.filter(
    (thread) =>
      thread.status !== 'working' &&
      mergedCleanup(thread, pullRequests[thread.id]) !== null
  )
  if (merged.length === 0) return null
  const label = m.cleanup_merged({ count: merged.length })

  const run = async () => {
    const confirmed = await confirm({
      title: label,
      description: m.cleanup_merged_confirm({ count: merged.length }),
      confirmLabel: m.clean_up_thread(),
      tone: 'destructive'
    })
    if (!confirmed) return
    setRunning(true)
    const reasons = new Set<string>()
    let kept = 0
    for (const thread of merged) {
      const error = await runCleanup('cleanUp', thread.id)
      if (error) {
        kept++
        reasons.add(cleanupErrorMessage(error))
      }
    }
    setRunning(false)
    if (kept > 0) {
      await notify({
        title: label,
        description: m.cleanup_merged_failed({
          count: kept,
          reasons: [...reasons].join(' ')
        }),
        confirmLabel: m.dialog_ok(),
        tone: 'warning'
      })
    }
  }

  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <button
            type='button'
            aria-label={label}
            disabled={running}
            onClick={() => void run()}
            className='text-muted-foreground hover:text-sidebar-foreground hover:bg-sidebar-accent focus-visible:ring-sidebar-ring mt-1 mr-1 flex size-5 shrink-0 cursor-pointer items-center justify-center rounded outline-none focus-visible:ring-2 disabled:opacity-50'
          />
        }
      >
        <HugeiconsIcon icon={CleanIcon} strokeWidth={2} className='size-3.5' />
      </TooltipTrigger>
      <TooltipContent side='right'>{label}</TooltipContent>
    </Tooltip>
  )
}
