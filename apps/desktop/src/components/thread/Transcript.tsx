import { cn, formatDuration } from '@hoshi/utils'
import { CHAT_LANE } from '@/lib/chat-lane'
import { m } from '@/paraglide/messages'
import { getLocale } from '@/paraglide/runtime'
import { HugeiconsIcon } from '@hugeicons/react'
import type { TurnFailure } from '@/bindings/TurnFailure'
import { Markdown } from '@/components/markdown/Markdown'
import { RequestRow } from './RequestRow'
import { ApprovalBadge, ApprovalsContext } from './ApprovalBadge'
import { approvalsByCall } from '@/lib/requests'
import { SubagentRow } from './SubagentRow'
import { TaskListRow } from './TaskListRow'
import { UserMessage } from './UserMessage'
import { useWholeToolCall } from './use-whole-tool-call'
import { DiffView } from '@/components/panel/DiffView'
import { TranscriptMinimap } from './TranscriptMinimap'
import { useTranscriptStore } from '@/state/transcript-store'
import { useAppStore } from '@/state/app-store'
import { useStoppingStore } from '@/state/stopping-store'
import {
  memo,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState
} from 'react'
import { Button } from '@hoshi/ui/components/button'
import {
  AiBrain01Icon,
  ArrowDown01Icon,
  ArrowRight01Icon,
  Cancel01Icon,
  Clock01Icon,
  CommandLineIcon,
  File01Icon,
  FileEditIcon,
  Globe02Icon,
  Loading03Icon,
  Search01Icon,
  StopCircleIcon,
  Wrench01Icon
} from '@hugeicons/core-free-icons'
import {
  activeEntry,
  foldTurns,
  groupTranscript,
  isSubagentCall,
  isToolCall,
  reasoningPreview,
  summarizeToolCalls,
  toolAction,
  toolName,
  toolTarget,
  withStreaming,
  type ToolAction,
  type ToolCallItem,
  type ToolSummaryPart,
  type TranscriptRow,
  type TranscriptItem,
  type WorkItem
} from '@/lib/tool-calls'

/** Within this distance of the bottom, new content keeps the view pinned to it. */
const STICK_TO_BOTTOM_PX = 80

/** Off-screen rows skip rendering but keep their last height, so scrolling never jumps. */
const ROW = '[contain-intrinsic-size:auto_2.5rem] [content-visibility:auto]'

const ENTRY_SUMMARY =
  'text-muted-foreground hover:text-foreground flex min-h-6 cursor-default list-none items-center gap-2 text-sm select-none [&::-webkit-details-marker]:hidden'

export function Transcript({
  working,
  cwd
}: {
  working: boolean
  cwd: string | null
}) {
  const items = useTranscriptStore((state) => state.items)
  const streaming = useTranscriptStore((state) => state.streaming)
  const rows = useMemo(
    () => foldTurns(groupTranscript(withStreaming(items, streaming)), working),
    [items, streaming, working]
  )
  const approvals = useMemo(() => approvalsByCall(items), [items])
  const liveTasks = working
    ? items.filter((item) => item.content.kind === 'taskList').at(-1)?.id
    : undefined
  const scrollRef = useRef<HTMLDivElement>(null)
  const listRef = useRef<HTMLOListElement>(null)
  const olderRef = useRef<HTMLLIElement>(null)
  const pinned = useRef(true)
  const hasOlder = useTranscriptStore((state) => state.hasOlder)
  const loadOlder = useTranscriptStore((state) => state.loadOlder)
  /** The scroll height before older items went in above, to keep the view where it was. */
  const heightBeforeOlder = useRef<number | null>(null)
  const firstId = items[0]?.id

  useLayoutEffect(() => {
    const element = scrollRef.current
    const before = heightBeforeOlder.current
    if (!element || before === null || firstId === undefined) return
    heightBeforeOlder.current = null
    element.scrollTop += element.scrollHeight - before
  }, [firstId])

  useEffect(() => {
    const element = scrollRef.current
    const sentinel = olderRef.current
    if (!element || !sentinel || !hasOlder) return
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (!entry?.isIntersecting) return
        heightBeforeOlder.current = element.scrollHeight
        void loadOlder()
      },
      { root: element, rootMargin: '800px 0px 0px 0px' }
    )
    observer.observe(sentinel)
    return () => observer.disconnect()
  }, [hasOlder, loadOlder, firstId])
  const [atEnd, setAtEnd] = useState(true)

  // Scroll position is DOM state: follow new content only if the reader was at the bottom.
  useLayoutEffect(() => {
    const element = scrollRef.current
    if (element && pinned.current) element.scrollTop = element.scrollHeight
  })

  // Rows settle to their real height only once on screen, after the scroll that revealed them.
  useEffect(() => {
    const element = scrollRef.current
    const list = listRef.current
    if (!element || !list) return
    const observer = new ResizeObserver(() => {
      if (pinned.current) element.scrollTop = element.scrollHeight
    })
    observer.observe(element)
    observer.observe(list)
    return () => observer.disconnect()
  }, [])

  const jumpTo = (id: string) => {
    const target = listRef.current?.querySelector(
      `[data-item-id="${CSS.escape(id)}"]`
    )
    if (!target) return
    pinned.current = false
    target.scrollIntoView({ block: 'start', behavior: 'smooth' })
  }

  const scrollToEnd = () => {
    const element = scrollRef.current
    if (!element) return
    pinned.current = true
    element.scrollTo({ top: element.scrollHeight, behavior: 'smooth' })
  }

  return (
    <div className='relative flex min-h-0 flex-1 flex-col'>
      <div
        ref={scrollRef}
        onScroll={(event) => {
          const element = event.currentTarget
          pinned.current =
            element.scrollHeight - element.scrollTop - element.clientHeight <
            STICK_TO_BOTTOM_PX
          setAtEnd(pinned.current)
        }}
        className='min-h-0 flex-1 overflow-y-auto'
      >
        <ol
          ref={listRef}
          className={cn(CHAT_LANE, 'flex flex-col gap-4 px-4 py-6')}
        >
          {hasOlder ? <li ref={olderRef} aria-hidden className='h-px' /> : null}
          <ApprovalsContext.Provider value={approvals}>
            <Rows rows={rows} cwd={cwd} liveTasks={liveTasks} />
          </ApprovalsContext.Provider>
          {working ? (
            <li>
              <TurnStatus />
            </li>
          ) : null}
        </ol>
      </div>
      <TranscriptMinimap
        rows={rows}
        scrollRef={scrollRef}
        listRef={listRef}
        onJump={jumpTo}
      />
      {atEnd ? null : (
        <div className='pointer-events-none absolute inset-x-4 bottom-1 z-10'>
          <div className={cn(CHAT_LANE, 'flex justify-center')}>
            <Button
              size='xs'
              variant='outline'
              onPointerDown={(event) => event.preventDefault()}
              onClick={scrollToEnd}
              className='bg-background/70 dark:bg-background/70 pointer-events-auto rounded-full shadow-sm backdrop-blur-md'
            >
              <HugeiconsIcon icon={ArrowDown01Icon} strokeWidth={2} />
              {m.scroll_to_end()}
            </Button>
          </div>
        </div>
      )}
    </div>
  )
}

type RowsProps = {
  rows: TranscriptRow[]
  cwd: string | null
  /** The task list of the running turn, which shows what is being worked on. */
  liveTasks: string | undefined
}

function Rows({ rows, cwd, liveTasks }: RowsProps) {
  return rows.map((row) => {
    switch (row.kind) {
      case 'item':
        return (
          <li
            key={row.item.id}
            data-item-id={row.item.id}
            className={cn(ROW, 'scroll-mt-6')}
          >
            <Item item={row.item} cwd={cwd} live={row.item.id === liveTasks} />
          </li>
        )
      case 'work':
        return (
          <li key={row.id} className={ROW}>
            <WorkGroup entries={row.entries} cwd={cwd} />
          </li>
        )
      default:
        return (
          <li key={row.id} className={ROW}>
            <TurnFold
              rows={row.rows}
              durationMs={row.durationMs}
              cwd={cwd}
              liveTasks={liveTasks}
            />
          </li>
        )
    }
  })
}

/** A finished turn's work, folded behind one line until opened. */
function TurnFold({
  durationMs,
  ...rows
}: RowsProps & { durationMs: number | null }) {
  const [open, setOpen] = useState(false)
  return (
    <div>
      <button
        type='button'
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className='group/fold text-muted-foreground hover:text-foreground focus-visible:ring-ring flex min-h-6 items-center gap-2 rounded-md text-sm outline-none focus-visible:ring-2'
      >
        <HugeiconsIcon
          icon={Clock01Icon}
          strokeWidth={2}
          className='size-4 shrink-0'
        />
        {durationMs === null
          ? m.worked()
          : m.worked_for({ duration: formatDuration(durationMs, getLocale()) })}
        <HugeiconsIcon
          icon={ArrowRight01Icon}
          strokeWidth={2}
          className={cn(
            'size-3.5 shrink-0 opacity-0 transition-transform group-hover/fold:opacity-100',
            open && 'rotate-90 opacity-100'
          )}
        />
      </button>
      {open ? (
        <ol className='border-border/60 ms-2 mt-2 flex flex-col gap-4 border-s ps-4'>
          <Rows {...rows} />
        </ol>
      ) : null}
    </div>
  )
}

const Item = memo(function Item({
  item,
  cwd,
  live
}: {
  item: TranscriptItem
  cwd: string | null
  /** The turn this item belongs to is still running. */
  live: boolean
}) {
  const { content } = item
  switch (content.kind) {
    case 'userMessage':
      return (
        <UserMessage text={content.text} attachments={content.attachments} />
      )
    case 'assistantMessage':
      return <Markdown text={content.text} />
    case 'reasoning':
      return (
        <ReasoningRow text={content.text} streaming={item.streaming === true} />
      )
    case 'toolCall':
      return isSubagentCall(item) ? (
        <SubagentRow
          item={item}
          renderRows={(rows) => (
            <Rows rows={rows} cwd={cwd} liveTasks={undefined} />
          )}
        />
      ) : (
        <ToolCallRow item={{ ...item, content }} cwd={cwd} />
      )
    case 'turnFailed':
      return (
        <p className='text-destructive text-sm'>
          {failureMessage(content.reason)}
        </p>
      )
    case 'turnInterrupted':
      return (
        <div className='text-muted-foreground flex items-center gap-3 text-xs'>
          <span className='bg-border h-px flex-1' />
          <span className='flex items-center gap-1.5'>
            <HugeiconsIcon
              icon={StopCircleIcon}
              strokeWidth={2}
              className='size-3.5'
            />
            {m.turn_interrupted()}
          </span>
          <span className='bg-border h-px flex-1' />
        </div>
      )
    case 'approval':
    case 'questions':
    case 'proposedPlan':
      return <RequestRow content={content} cwd={cwd} />
    case 'taskList':
      return <TaskListRow tasks={content.tasks} working={live} />
  }
})

const ACTION_ICONS: Record<ToolAction, typeof CommandLineIcon> = {
  command: CommandLineIcon,
  edit: FileEditIcon,
  read: File01Icon,
  search: Search01Icon,
  web: Globe02Icon,
  other: Wrench01Icon
}

type WorkGroupProps = { entries: WorkItem[]; cwd: string | null }

/**
 * A run of tool calls and reasoning, folded into one line that names what they did.
 * What is still running shows on that same line, so the row never changes height.
 */
const WorkGroup = memo(function WorkGroup({ entries, cwd }: WorkGroupProps) {
  const [open, setOpen] = useState(false)
  const [first] = entries
  if (entries.length === 1 && first)
    return <WorkEntry entry={first} cwd={cwd} />

  const calls = entries.filter(isToolCall)
  const active = activeEntry(entries)
  const last = calls.at(-1)?.content
  const failed = !active && last?.isError === true && !last.interrupted
  const actions = new Set(calls.map((call) => toolAction(call.content.name)))
  const [onlyAction] = actions
  const icon =
    calls.length === 0
      ? AiBrain01Icon
      : ACTION_ICONS[actions.size === 1 && onlyAction ? onlyAction : 'other']

  return (
    <div>
      <button
        type='button'
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className='group/tools text-muted-foreground hover:text-foreground focus-visible:ring-ring flex min-h-6 w-full min-w-0 items-center gap-2 rounded-md text-left text-sm outline-none focus-visible:ring-2'
      >
        <HugeiconsIcon
          icon={active ? Loading03Icon : failed ? Cancel01Icon : icon}
          strokeWidth={2}
          aria-label={
            active ? m.tool_running() : failed ? m.tool_failed() : undefined
          }
          className={cn(
            'size-4 shrink-0',
            active && 'animate-spin',
            failed && 'text-destructive'
          )}
        />
        <span className='shrink-0'>
          {calls.length > 0
            ? summaryText(summarizeToolCalls(calls))
            : m.reasoning_thought()}
        </span>
        {active ? (
          <span
            className={cn(
              'min-w-0 truncate opacity-70',
              active.content.kind === 'toolCall' && 'font-mono text-xs'
            )}
          >
            {active.content.kind === 'toolCall'
              ? toolTarget(active.content.name, active.content.input, cwd)
              : reasoningPreview(active.content.text, true)}
          </span>
        ) : null}
        <HugeiconsIcon
          icon={ArrowRight01Icon}
          strokeWidth={2}
          className={cn(
            'size-3.5 shrink-0 opacity-0 transition-transform group-hover/tools:opacity-100',
            open && 'rotate-90 opacity-100'
          )}
        />
      </button>
      {open ? (
        <ol className='border-border/60 ms-2 mt-1 flex flex-col border-s ps-2.5'>
          {entries.map((entry) => (
            <li key={entry.id}>
              <WorkEntry entry={entry} cwd={cwd} />
            </li>
          ))}
        </ol>
      ) : null}
    </div>
  )
}, sameWorkGroup)

function sameWorkGroup(previous: WorkGroupProps, next: WorkGroupProps) {
  return (
    previous.cwd === next.cwd &&
    previous.entries.length === next.entries.length &&
    previous.entries.every((entry, index) => entry === next.entries[index])
  )
}

function WorkEntry({ entry, cwd }: { entry: WorkItem; cwd: string | null }) {
  const { content } = entry
  return content.kind === 'toolCall' ? (
    <ToolCallRow item={{ ...entry, content }} cwd={cwd} />
  ) : (
    <ReasoningRow text={content.text} streaming={entry.streaming === true} />
  )
}

/** The model's reasoning, folded into one line until opened. */
function ReasoningRow({
  text,
  streaming
}: {
  text: string
  streaming: boolean
}) {
  const [open, setOpen] = useState(false)
  return (
    <details
      className='group/call'
      onToggle={(event) => setOpen(event.currentTarget.open)}
    >
      <summary className={ENTRY_SUMMARY}>
        <HugeiconsIcon
          icon={AiBrain01Icon}
          strokeWidth={2}
          className={cn('size-4 shrink-0', streaming && 'animate-pulse')}
        />
        <span className='text-foreground shrink-0 font-medium'>
          {streaming ? m.reasoning_thinking() : m.reasoning_thought()}
        </span>
        <span className='min-w-0 truncate'>
          {reasoningPreview(text, streaming)}
        </span>
        <HugeiconsIcon
          icon={ArrowRight01Icon}
          strokeWidth={2}
          className='size-3.5 shrink-0 opacity-0 group-open/call:rotate-90 group-open/call:opacity-100 group-hover/call:opacity-100'
        />
      </summary>
      {open ? (
        <div className='bg-muted/40 text-muted-foreground ms-6 mt-1 mb-2 max-h-80 overflow-auto rounded-md px-3 py-2'>
          <Markdown text={text} className='text-xs' />
        </div>
      ) : null}
    </details>
  )
}

const SUMMARY_MESSAGES: Record<
  ToolSummaryPart['action'],
  (inputs: { count: number }) => string
> = {
  command: m.tool_summary_command,
  edit: m.tool_summary_edit,
  read: m.tool_summary_read,
  search: m.tool_summary_search,
  web: m.tool_summary_web,
  other: m.tool_summary_other,
  others: m.tool_summary_others
}

function summaryText(parts: ToolSummaryPart[]) {
  const locale = getLocale()
  const sentence = new Intl.ListFormat(locale, { type: 'conjunction' }).format(
    parts.map(({ action, count }) => SUMMARY_MESSAGES[action]({ count }))
  )
  return sentence.charAt(0).toLocaleUpperCase(locale) + sentence.slice(1)
}

function ToolCallRow({
  item,
  cwd
}: {
  item: ToolCallItem
  cwd: string | null
}) {
  const { name, input, output, isError, detail, interrupted } = item.content
  const [open, setOpen] = useState(false)
  const whole = useWholeToolCall(item, open)
  const diff = whole.detail?.kind === 'fileChange' ? whole.detail.diff : null
  const running = output === null && !interrupted
  const live = useTranscriptStore((state) =>
    running && open ? state.streaming[item.id]?.text : undefined
  )
  const target = toolTarget(name, input, cwd)
  const shown = toolName(name)
  const change = detail?.kind === 'fileChange' ? detail : null
  const expandable = Boolean(target || output || change || running)
  return (
    <details
      className='group/call'
      onToggle={(event) => setOpen(event.currentTarget.open)}
    >
      <summary className={ENTRY_SUMMARY}>
        <HugeiconsIcon
          icon={
            running
              ? Loading03Icon
              : interrupted
                ? StopCircleIcon
                : isError
                  ? Cancel01Icon
                  : ACTION_ICONS[toolAction(name)]
          }
          strokeWidth={2}
          aria-label={
            running
              ? m.tool_running()
              : interrupted
                ? m.tool_stopped()
                : isError
                  ? m.tool_failed()
                  : undefined
          }
          className={cn(
            'size-4 shrink-0',
            running && 'animate-spin',
            isError && 'text-destructive'
          )}
        />
        <span className='text-foreground shrink-0 font-medium'>
          {shown.label}
        </span>
        {shown.source ? (
          <span className='shrink-0 text-xs'>{shown.source}</span>
        ) : null}
        <span className='min-w-0 truncate font-mono text-xs'>{target}</span>
        <ApprovalBadge callId={item.id} />
        {change ? (
          <span className='shrink-0 font-mono text-xs tabular-nums'>
            <span className='text-pr-open'>+{change.additions}</span>{' '}
            <span className='text-status-error'>−{change.deletions}</span>
          </span>
        ) : null}
        {expandable ? (
          <HugeiconsIcon
            icon={ArrowRight01Icon}
            strokeWidth={2}
            className='size-3.5 shrink-0 opacity-0 group-open/call:rotate-90 group-open/call:opacity-100 group-hover/call:opacity-100'
          />
        ) : null}
      </summary>
      {expandable ? (
        <div className='bg-muted/40 ms-6 mt-1 mb-2 overflow-hidden rounded-md font-mono text-xs'>
          {target ? (
            <pre className='text-foreground/85 max-h-40 overflow-auto px-3 py-2 break-all whitespace-pre-wrap'>
              {target}
            </pre>
          ) : null}
          {change && !isError ? (
            diff ? (
              <div className='border-border/60 max-h-96 overflow-auto border-t py-1'>
                <DiffView text={diff} />
              </div>
            ) : null
          ) : output ? (
            <pre
              className={cn(
                'text-muted-foreground max-h-80 overflow-auto px-3 py-2 whitespace-pre-wrap',
                target && 'border-border/60 border-t'
              )}
            >
              {whole.output}
            </pre>
          ) : live ? (
            <LiveOutput text={live} framed={Boolean(target)} />
          ) : null}
        </div>
      ) : null}
    </details>
  )
}

/** A running command's output so far, following its end unless scrolled up. */
function LiveOutput({ text, framed }: { text: string; framed: boolean }) {
  const ref = useRef<HTMLPreElement>(null)
  const following = useRef(true)
  useLayoutEffect(() => {
    const element = ref.current
    if (element && following.current) element.scrollTop = element.scrollHeight
  })
  return (
    <pre
      ref={ref}
      onScroll={(event) => {
        const element = event.currentTarget
        following.current =
          element.scrollHeight - element.scrollTop - element.clientHeight < 8
      }}
      className={cn(
        'text-muted-foreground max-h-80 overflow-auto px-3 py-2 whitespace-pre-wrap',
        framed && 'border-border/60 border-t'
      )}
    >
      {text}
    </pre>
  )
}

/** How long the running turn has worked, or how long it has been stopping. */
function TurnStatus() {
  const turnId = useAppStore(
    (state) =>
      state.threads.find((thread) => thread.id === state.selectedThreadId)
        ?.activeTurnId
  )
  const threadId = useAppStore((state) => state.selectedThreadId)
  const stoppingSince = useStoppingStore((state) =>
    threadId ? state.since[threadId] : undefined
  )
  const startedAt = useTranscriptStore((state) => {
    const asked = state.items.find((item) => item.id === turnId)
    return asked ? Date.parse(asked.createdAt) : undefined
  })
  const [now, setNow] = useState(Date.now)
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(timer)
  }, [])

  const since = stoppingSince ?? startedAt
  const duration =
    since === undefined || Number.isNaN(since)
      ? null
      : formatDuration(now - since, getLocale())
  const stopping = stoppingSince !== undefined
  return (
    <span className='text-muted-foreground flex items-center gap-2 text-sm'>
      <HugeiconsIcon
        icon={stopping ? StopCircleIcon : Loading03Icon}
        strokeWidth={2}
        className={cn('size-4 shrink-0', !stopping && 'animate-spin')}
      />
      {stopping
        ? m.stopping_for({ duration: duration ?? '' })
        : duration === null
          ? m.working()
          : m.working_for({ duration })}
    </span>
  )
}

function failureMessage(reason: TurnFailure) {
  switch (reason.code) {
    case 'providerDisabled':
      return m.turn_failed_provider_disabled()
    case 'spawnFailed':
      return m.turn_failed_spawn()
    case 'processExited':
      return m.turn_failed_process_exited()
    case 'providerError':
      return m.turn_failed_provider_error()
  }
}
