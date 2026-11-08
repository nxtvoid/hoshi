import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { useTranscriptStore } from '@/state/transcript-store'
import {
  Alert02Icon,
  BotIcon,
  CheckmarkCircle02Icon,
  CommandLineIcon
} from '@hugeicons/core-free-icons'
import {
  isSubagentCall,
  subagentInput,
  toolName,
  toolTarget
} from '@/lib/tool-calls'

/**
 * A task that ran in the background ended, which is what the agent answers next. Opens on
 * the call that started it.
 */
export function BackgroundTaskRow({
  toolCallId,
  failed,
  cwd
}: {
  toolCallId: string | null
  failed: boolean
  cwd: string | null
}) {
  const call = useTranscriptStore((state) =>
    toolCallId
      ? state.items.find(
          (item) => item.id === toolCallId && item.content.kind === 'toolCall'
        )
      : undefined
  )
  const subagent = call ? isSubagentCall(call) : false
  const content = call?.content.kind === 'toolCall' ? call.content : null
  const title = !content
    ? m.background_task()
    : subagent
      ? subagentInput(content.input).description ||
        subagentInput(content.input).kind
      : toolTarget(content.name, content.input, cwd) ||
        toolName(content.name).label

  return (
    <button
      type='button'
      disabled={!call}
      onClick={() =>
        document
          .querySelector(`[data-item-id="${CSS.escape(toolCallId ?? '')}"]`)
          ?.scrollIntoView({ block: 'center', behavior: 'smooth' })
      }
      className='text-muted-foreground hover:text-foreground focus-visible:ring-ring flex min-h-6 w-full min-w-0 items-center gap-2 rounded-md text-left text-sm outline-none focus-visible:ring-2 disabled:pointer-events-none'
    >
      <HugeiconsIcon
        icon={subagent ? BotIcon : CommandLineIcon}
        strokeWidth={2}
        className='size-4 shrink-0'
      />
      <span className='text-foreground/80 min-w-0 truncate font-medium'>
        {title}
      </span>
      <span className='flex shrink-0 items-center gap-1 text-xs'>
        <HugeiconsIcon
          icon={failed ? Alert02Icon : CheckmarkCircle02Icon}
          strokeWidth={2}
          className={cn('size-3.5', failed && 'text-destructive')}
        />
        {failed ? m.background_task_failed() : m.background_task_done()}
      </span>
    </button>
  )
}
