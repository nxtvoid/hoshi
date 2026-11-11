import { useEffect, useMemo, useRef, useState } from 'react'
import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { CHAT_LANE } from '@/lib/chat-lane'
import { AgentStatus } from './AgentLine'
import { stableRows } from '@/lib/stable-rows'
import { subagentState } from '@/lib/agents'
import { subagentRun } from '@/lib/model-runs'
import { TranscriptFrame } from './Transcript'
import { EFFORT_LABELS } from './effort-labels'
import { approvalsByCall } from '@/lib/requests'
import { useAppStore } from '@/state/app-store'
import { HugeiconsIcon } from '@hugeicons/react'
import { subagentRows } from '@/lib/subagent-rows'
import { Button } from '@hoshi/ui/components/button'
import { useModelsStore } from '@/state/models-store'
import { useWholeToolCall } from './use-whole-tool-call'
import { Markdown } from '@/components/markdown/Markdown'
import { useSubagentStore } from '@/state/subagent-store'
import { ClaudeIcon } from '@/components/brand/ProviderIcon'
import { itemById, useTranscriptStore } from '@/state/transcript-store'
import {
  isSubagentCall,
  subagentInput,
  subagentTranscriptId,
  type ToolCallItem,
  type TranscriptRow
} from '@/lib/tool-calls'
import { ArrowUpLeft01Icon, BotIcon } from '@hugeicons/core-free-icons'

/**
 * The call that started the subagent `depth` levels into the ones being shown: in the
 * thread's transcript for the first, in the transcript of the one before for the others.
 */
function useSubagentCall(threadId: string, depth: number) {
  const id = useSubagentStore((state) => state.viewing[depth])
  const owner = useSubagentStore((state) => state.viewing[depth - 1])
  const own = useTranscriptStore((state) =>
    id && !owner && state.threadId === threadId
      ? itemById(state.items, id)
      : undefined
  )
  const nested = useSubagentStore((state) =>
    id && owner
      ? state.transcripts[subagentTranscriptId(threadId, owner)]?.find(
          (item) => item.id === id
        )
      : undefined
  )
  const item = own ?? nested
  return item && isSubagentCall(item) ? item : undefined
}

function titleOf(item: ToolCallItem | undefined) {
  return (item && subagentInput(item.content.input).description) || m.subagent()
}

/** The name of the subagent shown in place of the thread. */
export function SubagentTitle({ threadId }: { threadId: string }) {
  const depth = useSubagentStore((state) => state.viewing.length - 1)
  return titleOf(useSubagentCall(threadId, depth))
}

function KeepLoaded({ transcriptId }: { transcriptId: string }) {
  const open = useSubagentStore((state) => state.open)
  const close = useSubagentStore((state) => state.close)
  useEffect(() => {
    void open(transcriptId)
    return () => close(transcriptId)
  }, [transcriptId, open, close])
  return null
}

/**
 * A subagent in place of the thread that started it: what it was asked, what it did and
 * what it reported. It runs on its own, so a bar stands where the composer would be.
 */
export function SubagentView({
  threadId,
  threadTitle,
  cwd
}: {
  threadId: string
  threadTitle: string
  cwd: string | null
}) {
  const viewing = useSubagentStore((state) => state.viewing)
  const item = useSubagentCall(threadId, viewing.length - 1)
  const owner = useSubagentCall(threadId, viewing.length - 2)
  return (
    <>
      {viewing.map((id) => (
        <KeepLoaded
          key={id}
          transcriptId={subagentTranscriptId(threadId, id)}
        />
      ))}
      {item ? (
        <SubagentTranscript
          key={item.id}
          item={item}
          parent={owner ? titleOf(owner) : threadTitle}
          cwd={cwd}
        />
      ) : (
        <div className='flex-1' />
      )}
      <div className='relative px-4 pb-3'>
        <div
          aria-hidden
          className='pointer-events-none absolute inset-x-4 bottom-full'
        >
          <div
            className={cn(
              CHAT_LANE,
              'from-background h-8 bg-linear-to-t to-transparent'
            )}
          />
        </div>
        <SubagentBar key={item?.id} item={item} />
      </div>
    </>
  )
}

function SubagentTranscript({
  item,
  parent,
  cwd
}: {
  item: ToolCallItem
  parent: string
  cwd: string | null
}) {
  const items = useSubagentStore(
    (state) => state.transcripts[subagentTranscriptId(item.threadId, item.id)]
  )
  const threadItems = useTranscriptStore((state) => state.items)
  const approvals = useMemo(() => approvalsByCall(threadItems), [threadItems])
  const progress = useTranscriptStore((state) => state.toolOutput[item.id])
  const whole = useWholeToolCall(item, true)
  const running = subagentState(item.content) === 'running'
  const report = item.content.interrupted ? null : whole.output
  const { prompt } = subagentInput(item.content.input)
  const shown = useRef<TranscriptRow[]>([])
  const rows = useMemo(() => {
    const next = stableRows(
      shown.current,
      subagentRows(item, items ?? [], report, running)
    )
    shown.current = next
    return next
  }, [item, items, report, running])

  return (
    <TranscriptFrame
      rows={rows}
      approvals={approvals}
      cwd={cwd}
      lead={
        <>
          <div className='text-muted-foreground flex items-center gap-3 text-xs'>
            <span className='bg-border h-px flex-1' />
            <span className='flex min-w-0 items-center gap-1.5'>
              <HugeiconsIcon
                icon={BotIcon}
                strokeWidth={2}
                className='size-3.5 shrink-0'
              />
              <span className='truncate'>{m.subagent_of({ parent })}</span>
            </span>
            <span className='bg-border h-px flex-1' />
          </div>
          {prompt ? (
            <details>
              <summary className='text-muted-foreground hover:text-foreground cursor-default list-none text-xs select-none [&::-webkit-details-marker]:hidden'>
                {m.subagent_prompt()}
              </summary>
              <div className='bg-muted/40 mt-1 max-h-60 overflow-auto rounded-md px-3 py-2'>
                <Markdown text={prompt} className='text-xs' />
              </div>
            </details>
          ) : null}
        </>
      }
    >
      {running ? (
        <span className='text-muted-foreground block truncate text-sm'>
          {progress || m.working()}
        </span>
      ) : items && rows.length === 0 ? (
        <p className='text-muted-foreground text-xs'>
          {m.subagent_no_activity()}
        </p>
      ) : null}
    </TranscriptFrame>
  )
}

/** Where the composer would be: what the subagent runs on, how it stands, and the way back. */
function SubagentBar({ item }: { item: ToolCallItem | undefined }) {
  const leave = useSubagentStore((state) => state.leave)
  const stopSubagent = useAppStore((state) => state.stopSubagent)
  const catalog = useModelsStore((state) => state.catalog)
  const [stopping, setStopping] = useState(false)
  const state = item ? subagentState(item.content) : null
  const run = item ? subagentRun(catalog, item.content.detail) : null
  return (
    <div className={cn(CHAT_LANE, '@container/bar')}>
      <div className='border-border bg-card flex min-h-12 items-center gap-3 rounded-2xl border py-2 ps-4 pe-2 text-sm shadow-xs'>
        <ClaudeIcon className='text-provider-claude size-4 shrink-0' />
        <span className='flex min-w-0 items-center gap-2'>
          <span className='text-foreground truncate font-medium'>
            {run?.model ?? m.subagent()}
          </span>
          {run?.effort ? (
            <span className='text-muted-foreground hidden shrink-0 @md/bar:inline'>
              {EFFORT_LABELS[run.effort]()}
            </span>
          ) : null}
        </span>
        {item && state ? (
          <AgentStatus
            state={state}
            startedAt={item.createdAt}
            endedAt={null}
          />
        ) : null}
        <span className='flex-1' />
        <span className='text-muted-foreground hidden shrink-0 @lg/bar:inline'>
          {m.subagent_runs_alone()}
        </span>
        {item && state === 'running' ? (
          <Button
            size='sm'
            variant='ghost'
            disabled={stopping}
            onClick={() => {
              setStopping(true)
              void stopSubagent(item.id)
            }}
            className='text-muted-foreground hover:text-foreground shrink-0'
          >
            {stopping ? m.background_task_stopping() : m.background_task_stop()}
          </Button>
        ) : null}
        <Button size='sm' variant='ghost' onClick={leave} className='shrink-0'>
          <HugeiconsIcon icon={ArrowUpLeft01Icon} strokeWidth={2} />
          <span className='sr-only @md/bar:not-sr-only'>
            {m.subagent_open_parent()}
          </span>
        </Button>
      </div>
    </div>
  )
}
