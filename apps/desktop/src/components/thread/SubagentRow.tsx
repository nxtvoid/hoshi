import { useEffect, useState } from 'react'
import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { Markdown } from '@/components/markdown/Markdown'
import { useAppStore } from '@/state/app-store'
import { useSubagentStore } from '@/state/subagent-store'
import { IconButton } from '@hoshi/ui/components/icon-button'
import { useTranscriptStore } from '@/state/transcript-store'
import { useWholeToolCall } from './use-whole-tool-call'
import {
  ArrowRight01Icon,
  BotIcon,
  Cancel01Icon,
  Loading03Icon,
  StopCircleIcon,
  StopIcon
} from '@hugeicons/core-free-icons'
import {
  groupTranscript,
  subagentInput,
  subagentTranscriptId,
  type ToolCallItem,
  type TranscriptRow
} from '@/lib/tool-calls'

/** A subagent the agent started: what it was asked, what it is doing, and what it found. */
export function SubagentRow({
  item,
  renderRows
}: {
  item: ToolCallItem
  /** Renders the subagent's own activity the way the thread renders its own. */
  renderRows: (rows: TranscriptRow[]) => React.ReactNode
}) {
  const [open, setOpen] = useState(false)
  const { output, isError, input, interrupted } = item.content
  const running = output === null && !interrupted
  const { description, kind, prompt } = subagentInput(input)
  const progress = useTranscriptStore((state) =>
    running ? state.streaming[item.id]?.text : undefined
  )
  const whole = useWholeToolCall(item, open)
  const stopSubagent = useAppStore((state) => state.stopSubagent)

  return (
    <div className='border-border/70 rounded-xl border'>
      <div className={cn('flex items-center gap-1', running && 'pe-1.5')}>
        <button
          type='button'
          aria-expanded={open}
          onClick={() => setOpen(!open)}
          className='group/agent hover:bg-muted/40 focus-visible:ring-ring flex min-w-0 flex-1 items-center gap-2.5 rounded-xl px-3 py-2 text-left text-sm outline-none focus-visible:ring-2'
        >
          <HugeiconsIcon
            icon={
              running
                ? Loading03Icon
                : interrupted
                  ? StopCircleIcon
                  : isError
                    ? Cancel01Icon
                    : BotIcon
            }
            strokeWidth={2}
            aria-label={
              running ? m.tool_running() : isError ? m.tool_failed() : undefined
            }
            className={cn(
              'text-muted-foreground size-4 shrink-0',
              running && 'animate-spin',
              isError && 'text-destructive'
            )}
          />
          <span className='flex min-w-0 flex-1 flex-col'>
            <span className='flex min-w-0 items-center gap-2'>
              <span className='text-foreground truncate font-medium'>
                {description || m.subagent()}
              </span>
              {kind ? (
                <span className='text-muted-foreground shrink-0 text-xs'>
                  {kind}
                </span>
              ) : null}
            </span>
            {running && progress ? (
              <span className='text-muted-foreground truncate text-xs'>
                {progress}
              </span>
            ) : null}
          </span>
          <HugeiconsIcon
            icon={ArrowRight01Icon}
            strokeWidth={2}
            className={cn(
              'text-muted-foreground size-3.5 shrink-0 opacity-0 transition-transform group-hover/agent:opacity-100',
              open && 'rotate-90 opacity-100'
            )}
          />
        </button>
        {running ? (
          <IconButton
            label={m.subagent_stop()}
            onClick={() => void stopSubagent(item.id)}
            side='top'
          >
            <HugeiconsIcon icon={StopIcon} strokeWidth={2} />
          </IconButton>
        ) : null}
      </div>
      {open ? (
        <div className='border-border/70 flex flex-col gap-3 border-t px-3 py-3'>
          {prompt ? (
            <details className='group/prompt'>
              <summary className='text-muted-foreground hover:text-foreground cursor-default list-none text-xs select-none [&::-webkit-details-marker]:hidden'>
                {m.subagent_prompt()}
              </summary>
              <div className='bg-muted/40 mt-1 max-h-60 overflow-auto rounded-md px-3 py-2'>
                <Markdown text={prompt} className='text-xs' />
              </div>
            </details>
          ) : null}
          <SubagentActivity
            transcriptId={subagentTranscriptId(item.threadId, item.id)}
            renderRows={renderRows}
          />
          {whole.output && !interrupted ? (
            <div className='border-border/70 border-t pt-3'>
              <Markdown text={whole.output} className='text-sm' />
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}

function SubagentActivity({
  transcriptId,
  renderRows
}: {
  transcriptId: string
  renderRows: (rows: TranscriptRow[]) => React.ReactNode
}) {
  const items = useSubagentStore((state) => state.transcripts[transcriptId])
  const open = useSubagentStore((state) => state.open)
  const close = useSubagentStore((state) => state.close)

  useEffect(() => {
    void open(transcriptId)
    return () => close(transcriptId)
  }, [transcriptId, open, close])

  if (!items) return null
  if (items.length === 0) {
    return (
      <p className='text-muted-foreground text-xs'>
        {m.subagent_no_activity()}
      </p>
    )
  }
  return (
    <ol className='flex flex-col gap-2'>
      {renderRows(groupTranscript(items))}
    </ol>
  )
}
