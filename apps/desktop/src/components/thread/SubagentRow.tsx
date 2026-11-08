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
  ArrowDown01Icon,
  ArrowRight01Icon,
  BotIcon,
  Cancel01Icon,
  Loading03Icon,
  StopCircleIcon,
  StopIcon
} from '@hugeicons/core-free-icons'
import {
  groupTranscript,
  reasoningPreview,
  subagentInput,
  subagentTranscriptId,
  type ToolCallItem,
  type TranscriptRow
} from '@/lib/tool-calls'

/**
 * Subagents the agent started together, as one card that says how many still work, and
 * opens on each of them.
 */
export function SubagentGroup({
  items,
  renderRows
}: {
  items: ToolCallItem[]
  renderRows: (rows: TranscriptRow[]) => React.ReactNode
}) {
  const [open, setOpen] = useState(false)
  const working = items.filter(
    ({ content }) => content.output === null && !content.interrupted
  ).length
  return (
    <div className='border-border/70 rounded-xl border'>
      <button
        type='button'
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className='hover:bg-muted/40 focus-visible:ring-ring flex w-full min-w-0 items-center gap-2.5 rounded-xl px-3 py-2 text-left text-sm outline-none focus-visible:ring-2'
      >
        <span className='flex shrink-0 -space-x-1.5'>
          {items.slice(0, 3).map((item) => (
            <span
              key={item.id}
              className='bg-muted ring-card text-muted-foreground flex size-5 items-center justify-center rounded-full ring-2'
            >
              <HugeiconsIcon
                icon={BotIcon}
                strokeWidth={2}
                className='size-3.5'
              />
            </span>
          ))}
        </span>
        <span className='flex min-w-0 flex-1 flex-col'>
          <span className='text-foreground truncate font-medium'>
            {m.subagents_count({ count: items.length })}
          </span>
          <span
            className={cn(
              'truncate text-xs',
              working > 0 ? 'text-status-working' : 'text-muted-foreground'
            )}
          >
            {working > 0
              ? m.subagents_working({ count: working })
              : m.subagents_done({ count: items.length })}
          </span>
        </span>
        <HugeiconsIcon
          icon={ArrowDown01Icon}
          strokeWidth={2}
          className={cn(
            'text-muted-foreground size-3.5 shrink-0 transition-transform',
            open && 'rotate-180'
          )}
        />
      </button>
      {open ? (
        <ol className='border-border/70 flex flex-col gap-2 border-t p-2'>
          {items.map((item) => (
            <li key={item.id} data-item-id={item.id}>
              <SubagentRow item={item} renderRows={renderRows} />
            </li>
          ))}
        </ol>
      ) : null}
    </div>
  )
}

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
            ) : !running && !interrupted && output ? (
              <span className='text-muted-foreground truncate text-xs'>
                {reasoningPreview(output, false)}
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
            result={interrupted ? null : whole.output}
            renderRows={renderRows}
          />
        </div>
      ) : null}
    </div>
  )
}

/** What the subagent did, then what it reported, unless its last message already says it. */
function SubagentActivity({
  transcriptId,
  result,
  renderRows
}: {
  transcriptId: string
  result: string | null
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
  const said = items
    .filter((entry) => entry.content.kind === 'assistantMessage')
    .at(-1)
  const repeated =
    said?.content.kind === 'assistantMessage' &&
    said.content.text.trim() === result?.trim()
  return (
    <>
      {items.length === 0 ? (
        <p className='text-muted-foreground text-xs'>
          {m.subagent_no_activity()}
        </p>
      ) : (
        <ol className='flex flex-col gap-2'>
          {renderRows(groupTranscript(items))}
        </ol>
      )}
      {result && !repeated ? (
        <div className='border-border/70 border-t pt-3'>
          <Markdown text={result} className='text-sm' />
        </div>
      ) : null}
    </>
  )
}
