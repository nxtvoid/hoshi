import { cn, formatDuration } from '@hoshi/utils'
import { CHAT_LANE } from '@/lib/chat-lane'
import { m } from '@/paraglide/messages'
import { getLocale } from '@/paraglide/runtime'
import { HugeiconsIcon } from '@hugeicons/react'
import type { TurnFailure } from '@/bindings/TurnFailure'
import { Markdown } from '@/components/markdown/Markdown'
import { RequestRow } from './RequestRow'
import { ApprovalBadge, ApprovalsContext } from './ApprovalBadge'
import { type ApprovalContent, approvalsByCall } from '@/lib/requests'
import { AgentGroup } from './AgentGroup'
import { SubagentRow } from './SubagentRow'
import { TaskListRow } from './TaskListRow'
import { TurnChangesCard } from './TurnChangesCard'
import { RewindButton } from './RewindButton'
import { CompactionRow } from './Compaction'
import { isCompactCommand } from '@/lib/compaction'
import { PullRequestUpdateRow } from './PullRequestUpdateRow'
import { BackgroundTaskRow } from './BackgroundTaskRow'
import { BackgroundWork } from './BackgroundWork'
import { UserMessage } from './UserMessage'
import { useWholeToolCall } from './use-whole-tool-call'
import { DiffView } from '@/components/panel/DiffView'
import { TranscriptMinimap } from './TranscriptMinimap'
import { itemById, useTranscriptStore } from '@/state/transcript-store'
import { useAppStore } from '@/state/app-store'
import { useStoppingStore } from '@/state/stopping-store'
import { inFlight } from '@/lib/agents'
import { foldTaskRuns } from '@/lib/task-runs'
import { rowKey, stableRows } from '@/lib/stable-rows'
import { useRequestStore } from '@/state/request-store'
import { useBusyThreads } from './use-busy-threads'
import { DelegationRow, TaskNote, TaskWaits } from './DelegationRows'
import {
  createContext,
  memo,
  type ReactNode,
  type RefObject,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState
} from 'react'
import {
  MessageScroller,
  MessageScrollerButton,
  MessageScrollerContent,
  MessageScrollerItem,
  MessageScrollerProvider,
  MessageScrollerViewport,
  useMessageScroller
} from '@hoshi/ui/components/message-scroller'
import {
  AiBrain01Icon,
  ArrowDown01Icon,
  ArrowRight01Icon,
  Cancel01Icon,
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
  isDelegation,
  staysInView,
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
  type FoldEnding,
  type TaskOutcome,
  type ToolSummaryPart,
  type TranscriptRow,
  type TranscriptItem,
  type TurnChanges,
  type WorkItem
} from '@/lib/tool-calls'

/** Off-screen rows skip rendering but keep their last height, so scrolling never jumps. */
const ROW = '[contain-intrinsic-size:auto_2.5rem] [content-visibility:auto]'
const MESSAGE_ROW =
  '[contain-intrinsic-size:auto_120px] [content-visibility:auto]'

const ENTRY_SUMMARY =
  'text-muted-foreground hover:text-foreground flex min-h-6 cursor-default list-none items-center gap-2 text-sm select-none [&::-webkit-details-marker]:hidden'

export const Transcript = memo(function Transcript({
  working,
  cwd
}: {
  working: boolean
  cwd: string | null
}) {
  const items = useTranscriptStore((state) => state.items)
  const streaming = useTranscriptStore((state) => state.streaming)
  const threadId = useTranscriptStore((state) => state.threadId)
  const pendingTasks = useRequestStore((state) => state.pendingTasks)
  const busy = useBusyThreads()
  const activeTasks = useAppStore((state) =>
    state.threads
      .filter(
        (thread) =>
          thread.parentThreadId === threadId &&
          inFlight(thread, pendingTasks, busy)
      )
      .map((thread) => thread.id)
      .join(' ')
  )
  const shownRows = useRef<TranscriptRow[]>([])
  const rows = useMemo(() => {
    const active = new Set(activeTasks.split(' '))
    const next = stableRows(
      shownRows.current,
      foldTaskRuns(
        foldTurns(
          groupTranscript(withStreaming(items, streaming)),
          working,
          staysInView
        ),
        (taskId) => active.has(taskId)
      )
    )
    shownRows.current = next
    return next
  }, [items, streaming, working, activeTasks])
  const approvals = useMemo(() => approvalsByCall(items), [items])
  const liveTasks = useMemo(
    () =>
      working
        ? items.filter((item) => item.content.kind === 'taskList').at(-1)?.id
        : undefined,
    [items, working]
  )
  const hasOlder = useTranscriptStore((state) => state.hasOlder)
  const loadOlder = useTranscriptStore((state) => state.loadOlder)

  return (
    <TranscriptFrame
      rows={rows}
      approvals={approvals}
      cwd={cwd}
      liveTasks={liveTasks}
      loadOlder={hasOlder ? loadOlder : undefined}
      firstId={items[0]?.id}
    >
      <BackgroundWork status={working ? <TurnStatus /> : null} />
      {working || !threadId ? null : <TaskWaits threadId={threadId} />}
    </TranscriptFrame>
  )
})

/**
 * Rows in the scroller every transcript shares, a thread's or a subagent's: it follows the
 * end while the reader is there, and jumps to a message from the minimap.
 */
export function TranscriptFrame({
  rows,
  approvals,
  cwd,
  liveTasks,
  loadOlder,
  firstId,
  lead,
  children
}: {
  rows: TranscriptRow[]
  approvals: Map<string, ApprovalContent>
  cwd: string | null
  liveTasks?: string
  /** Loads what came before the first row, while there is more, as the reader nears it. */
  loadOlder?: () => Promise<void>
  firstId?: string
  /** Before the rows. */
  lead?: ReactNode
  /** After the rows. */
  children?: ReactNode
}) {
  const scrollRef = useRef<HTMLDivElement>(null)
  const listRef = useRef<HTMLDivElement>(null)
  const olderRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const element = scrollRef.current
    const sentinel = olderRef.current
    if (!element || !sentinel || !loadOlder) return
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry?.isIntersecting) void loadOlder()
      },
      { root: element, rootMargin: '800px 0px 0px 0px' }
    )
    observer.observe(sentinel)
    return () => observer.disconnect()
  }, [loadOlder, firstId])

  return (
    <MessageScrollerProvider autoScroll>
      <MessageScroller className='flex-1'>
        <MessageScrollerViewport
          ref={scrollRef}
          aria-label={m.transcript_label()}
          onClickCapture={(event) => {
            const target = event.target as HTMLElement
            if (target.closest('[aria-expanded="false"]')) {
              keepInPlace(event.currentTarget)
            }
          }}
        >
          {loadOlder ? <div ref={olderRef} aria-hidden /> : null}
          <MessageScrollerContent
            ref={listRef}
            className={cn(CHAT_LANE, 'gap-4 px-4 pt-6 pb-12')}
          >
            {lead}
            <ApprovalsContext.Provider value={approvals}>
              <Rows rows={rows} cwd={cwd} liveTasks={liveTasks} />
            </ApprovalsContext.Provider>
            {children}
          </MessageScrollerContent>
        </MessageScrollerViewport>
        <Minimap rows={rows} scrollRef={scrollRef} listRef={listRef} />
        <div className='pointer-events-none absolute inset-x-4 bottom-1 z-10'>
          <div className={cn(CHAT_LANE, 'flex justify-center')}>
            <MessageScrollerButton
              size='xs'
              variant='outline'
              onPointerDown={(event) => event.preventDefault()}
              className='bg-background hover:bg-muted dark:bg-background dark:hover:bg-muted pointer-events-auto static translate-x-0 rounded-full shadow-sm'
            >
              <HugeiconsIcon icon={ArrowDown01Icon} strokeWidth={2} />
              {m.scroll_to_end()}
            </MessageScrollerButton>
          </div>
        </div>
      </MessageScroller>
    </MessageScrollerProvider>
  )
}

/** Where a jump to a message leaves it, below the top of the transcript. */
const JUMP_MARGIN_PX = 24

function Minimap({
  rows,
  scrollRef,
  listRef
}: {
  rows: TranscriptRow[]
  scrollRef: RefObject<HTMLDivElement | null>
  listRef: RefObject<HTMLDivElement | null>
}) {
  const { scrollToMessage } = useMessageScroller()
  return (
    <TranscriptMinimap
      rows={rows}
      scrollRef={scrollRef}
      listRef={listRef}
      onJump={(id) =>
        scrollToMessage(id, {
          behavior: 'smooth',
          scrollMargin: JUMP_MARGIN_PX
        })
      }
    />
  )
}

/** Whether rows render inside another row, as its list items, not as the transcript's own. */
const InRow = createContext(false)

function RowItem({
  messageId,
  itemId,
  className,
  children
}: {
  messageId: string
  itemId?: string
  className: string
  children: ReactNode
}) {
  const inRow = useContext(InRow)
  if (inRow) {
    return (
      <li data-item-id={itemId} className={className}>
        {children}
      </li>
    )
  }
  return (
    <MessageScrollerItem
      messageId={messageId}
      data-item-id={itemId}
      className={className}
    >
      <InRow.Provider value={true}>{children}</InRow.Provider>
    </MessageScrollerItem>
  )
}

/**
 * A row opening grows the transcript below the click, which should stay where it is. The
 * scroller lets go of the end only on wheel, touch or keys, and the browser's scroll
 * anchoring would keep the text under the row still instead, so both are told otherwise
 * until the row has opened.
 */
function keepInPlace(viewport: HTMLElement) {
  viewport.dispatchEvent(new WheelEvent('wheel', { bubbles: true }))
  viewport.style.overflowAnchor = 'none'
  requestAnimationFrame(() =>
    requestAnimationFrame(() => {
      viewport.style.overflowAnchor = ''
    })
  )
}

type RowsProps = {
  rows: TranscriptRow[]
  cwd: string | null
  /** The task list of the running turn, which shows what is being worked on. */
  liveTasks: string | undefined
}

function isAgent(row: TranscriptRow) {
  return (
    row.kind === 'item' && (isSubagentCall(row.item) || isDelegation(row.item))
  )
}

function Rows({ rows, cwd, liveTasks }: RowsProps) {
  const shown: React.ReactNode[] = []
  let agents: TranscriptRow[] = []
  const flushAgents = () => {
    const [first] = agents
    if (first?.kind === 'item' && agents.length > 1) {
      shown.push(<AgentsRow key={`agents:${first.item.id}`} rows={agents} />)
    } else if (first) {
      shown.push(
        <Row key={rowKey(first)} row={first} cwd={cwd} liveTasks={liveTasks} />
      )
    }
    agents = []
  }
  for (const row of rows) {
    if (isAgent(row)) {
      agents.push(row)
      continue
    }
    flushAgents()
    shown.push(
      <Row key={rowKey(row)} row={row} cwd={cwd} liveTasks={liveTasks} />
    )
  }
  flushAgents()
  return shown
}

type AgentsRowProps = { rows: TranscriptRow[] }

/** Subagents and delegated tasks started one after another, shown as one group. */
const AgentsRow = memo(function AgentsRow({ rows }: AgentsRowProps) {
  const items = rows.flatMap((row) =>
    row.kind === 'item' && (isSubagentCall(row.item) || isDelegation(row.item))
      ? [row.item]
      : []
  )
  return (
    <RowItem
      messageId={`agents:${items[0]?.id}`}
      itemId={items[0]?.id}
      className={cn(ROW, 'scroll-mt-6')}
    >
      <AgentGroup items={items} />
    </RowItem>
  )
}, sameAgents)

function sameAgents(previous: AgentsRowProps, next: AgentsRowProps) {
  return (
    previous.rows.length === next.rows.length &&
    previous.rows.every((row, index) => row === next.rows[index])
  )
}

const Row = memo(function Row({
  row,
  cwd,
  liveTasks
}: Omit<RowsProps, 'rows'> & { row: TranscriptRow }) {
  switch (row.kind) {
    case 'item':
      return (
        <RowItem
          messageId={row.item.id}
          itemId={row.item.id}
          className={cn(isMessage(row.item) ? MESSAGE_ROW : ROW, 'scroll-mt-6')}
        >
          <Item
            item={row.item}
            cwd={cwd}
            live={row.item.id === liveTasks}
            changes={row.changes}
            note={row.note}
          />
        </RowItem>
      )
    case 'work':
      return (
        <RowItem messageId={row.id} className={ROW}>
          <WorkGroup entries={row.entries} cwd={cwd} />
        </RowItem>
      )
    default:
      return (
        <RowItem messageId={row.id} className={ROW}>
          <TurnFold
            rows={row.rows}
            durationMs={row.durationMs}
            ending={row.ending}
            after={row.after}
            cwd={cwd}
            liveTasks={liveTasks}
          />
        </RowItem>
      )
  }
})

function isMessage(item: TranscriptItem) {
  return (
    item.content.kind === 'assistantMessage' ||
    item.content.kind === 'userMessage'
  )
}

/** A finished turn's work, folded behind one line until opened. */
function TurnFold({
  durationMs,
  ending,
  after,
  ...rows
}: RowsProps & {
  durationMs: number | null
  ending: FoldEnding
  after: 'start' | 'steer'
}) {
  const [open, setOpen] = useState(false)
  const duration =
    durationMs === null ? null : formatDuration(durationMs, getLocale())
  return (
    <div className='border-border/60 border-b pb-2'>
      <button
        type='button'
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className='text-muted-foreground hover:text-foreground focus-visible:ring-ring flex min-h-6 items-center gap-1 rounded-md text-sm tabular-nums outline-none focus-visible:ring-2'
      >
        {ending === 'stopped'
          ? duration === null
            ? m.stopped_response()
            : m.stopped_after({ duration })
          : after === 'steer'
            ? duration === null
              ? m.continued()
              : m.continued_for({ duration })
            : duration === null
              ? m.worked()
              : m.worked_for({ duration })}
        <HugeiconsIcon
          icon={ArrowRight01Icon}
          strokeWidth={2}
          className={cn(
            'size-3.5 shrink-0 transition-transform',
            open && 'rotate-90'
          )}
        />
      </button>
      {open ? (
        <ol className='border-border/60 ms-2 mt-2 mb-2 flex flex-col gap-4 border-s ps-4 [&>li]:[content-visibility:visible]'>
          <Rows {...rows} />
        </ol>
      ) : null}
    </div>
  )
}

const Item = memo(function Item({
  item,
  cwd,
  live,
  changes,
  note
}: {
  item: TranscriptItem
  cwd: string | null
  /** The turn this item belongs to is still running. */
  live: boolean
  /** What the turn changed, on its last answer. */
  changes?: TurnChanges
  /** A reply to a task's answer, while the work on its tasks goes on. */
  note?: TaskOutcome
}) {
  const { content } = item
  switch (content.kind) {
    case 'userMessage':
      if (isCompactCommand(content.text)) return null
      if (content.pullRequestUpdate) {
        return (
          <PullRequestUpdateRow
            update={content.pullRequestUpdate}
            text={content.text}
          />
        )
      }
      return content.taskResult ? null : (
        <UserMessage
          text={content.text}
          attachments={content.attachments}
          steer={content.steer}
          createdAt={item.createdAt}
          action={
            content.rewind ? (
              <RewindButton
                messageId={item.id}
                text={content.text}
                attachments={content.attachments}
              />
            ) : null
          }
        />
      )
    case 'assistantMessage':
      if (note) {
        return (
          <>
            <TaskNote
              text={content.text}
              outcome={note}
              streaming={item.streaming === true}
            />
            {changes ? <TurnChangesCard changes={changes} /> : null}
          </>
        )
      }
      return (
        <>
          <Markdown text={content.text} streaming={item.streaming === true} />
          {changes ? <TurnChangesCard changes={changes} /> : null}
        </>
      )
    case 'reasoning':
      return (
        <ReasoningRow text={content.text} streaming={item.streaming === true} />
      )
    case 'toolCall':
      if (isDelegation(item)) return <DelegationRow item={item} />
      return isSubagentCall(item) ? (
        <SubagentRow item={item} />
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
    case 'runEnded':
      return null
    case 'compaction':
      return <CompactionRow content={content} />
    case 'backgroundTaskEnded':
      return (
        <BackgroundTaskRow
          toolCallId={content.toolCallId}
          failed={content.failed}
          cwd={cwd}
        />
      )
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
          <Markdown text={text} streaming={streaming} className='text-xs' />
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
    running && open ? state.toolOutput[item.id] : undefined
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
    const asked = turnId ? itemById(state.items, turnId) : undefined
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
