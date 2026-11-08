import { cn } from '@hoshi/utils'
import { useEffect } from 'react'
import { m } from '@/paraglide/messages'
import { CHAT_LANE } from '@/lib/chat-lane'
import { useAppStore } from '@/state/app-store'
import { HugeiconsIcon } from '@hugeicons/react'
import { Button } from '@hoshi/ui/components/button'
import { Comment01Icon } from '@hugeicons/core-free-icons'
import { formatReviewThreads } from '@/lib/review-comments'
import { usePullRequestStore } from '@/state/pull-request-store'

/** Above the composer while the thread's pull request has unresolved review comments. */
export function ReviewCommentsBar({ threadId }: { threadId: string }) {
  const pullRequest = useAppStore((state) => state.pullRequests[threadId])
  const working = useAppStore(
    (state) =>
      state.threads.find((thread) => thread.id === threadId)?.status ===
      'working'
  )
  const sendMessage = useAppStore((state) => state.sendMessage)
  const detail = usePullRequestStore((state) => state.details[threadId])
  const load = usePullRequestStore((state) => state.load)
  const open = pullRequest?.state === 'open' || pullRequest?.state === 'draft'
  const count = open ? pullRequest.unresolvedComments : 0

  useEffect(() => {
    if (count > 0) void load(threadId)
  }, [count, threadId, load])

  if (!pullRequest || count === 0 || working) return null
  const unresolved = detail?.reviewThreads.filter((thread) => !thread.resolved)

  return (
    <div
      className={cn(
        CHAT_LANE,
        'border-border bg-muted/40 mb-2 flex items-center gap-2 rounded-xl border py-1.5 ps-3 pe-1.5 text-xs'
      )}
    >
      <HugeiconsIcon
        icon={Comment01Icon}
        strokeWidth={2}
        className='text-status-waiting size-3.5 shrink-0'
      />
      <span className='min-w-0 flex-1 truncate'>
        {m.review_comments_waiting({
          count,
          number: pullRequest.number
        })}
      </span>
      <Button
        size='xs'
        disabled={!unresolved || unresolved.length === 0}
        onClick={() => {
          if (!unresolved) return
          void sendMessage(
            m.address_comments_prompt({
              number: pullRequest.number,
              comments: formatReviewThreads(unresolved)
            })
          )
        }}
      >
        {m.address_comments()}
      </Button>
    </div>
  )
}
