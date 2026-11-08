import { cn } from '@hoshi/utils'
import { CHAT_LANE } from '@/lib/chat-lane'
import { m } from '@/paraglide/messages'
import { getLocale } from '@/paraglide/runtime'
import { HugeiconsIcon } from '@hugeicons/react'
import type { TurnFailure } from '@/bindings/TurnFailure'
import { Markdown } from '@/components/markdown/Markdown'
import { UserMessage } from './UserMessage'
import { TranscriptMinimap } from './TranscriptMinimap'
import { useTranscriptStore } from '@/state/transcript-store'
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
  CommandLineIcon,
  File01Icon,
  FileEditIcon,
  Globe02Icon,
  Loading03Icon,
  Search01Icon,
  Wrench01Icon
} from '@hugeicons/core-free-icons'
import {
  activeEntry,
  groupTranscript,
  isToolCall,
  reasoningPreview,
  summarizeToolCalls,
  toolAction,
  toolTarget,
  withStreaming,
  type ToolAction,
  type ToolCallContent,
  type ToolSummaryPart,
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
    () => groupTranscript(withStreaming(items, streaming)),
    [items, streaming]
  )
  const scrollRef = useRef<HTMLDivElement>(null)
  const listRef = useRef<HTMLOListElement>(null)
  const pinned = useRef(true)
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
          {rows.map((row) =>
            row.kind === 'item' ? (
              <li
                key={row.item.id}
                data-item-id={row.item.id}
                className={cn(ROW, 'scroll-mt-6')}
              >
                <Item item={row.item} />
              </li>
            ) : (
              <li key={row.id} className={ROW}>
                <WorkGroup entries={row.entries} cwd={cwd} />
              </li>
            )
          )}
          {working ? (
            <li className='text-muted-foreground text-sm'>{m.working()}</li>
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

const Item = memo(function Item({ item }: { item: TranscriptItem }) {
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
      return <ToolCallRow call={content} cwd={null} />
    case 'turnFailed':
      return (
        <p className='text-destructive text-sm'>
          {failureMessage(content.reason)}
        </p>
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
  const failed = !active && (calls.at(-1)?.content.isError ?? false)
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
  return entry.content.kind === 'toolCall' ? (
    <ToolCallRow call={entry.content} cwd={cwd} />
  ) : (
    <ReasoningRow
      text={entry.content.text}
      streaming={entry.streaming === true}
    />
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
  call,
  cwd
}: {
  call: ToolCallContent
  cwd: string | null
}) {
  const { name, input, output, isError } = call
  const running = output === null
  const target = toolTarget(name, input, cwd)
  const expandable = Boolean(target || output)
  return (
    <details className='group/call'>
      <summary className={ENTRY_SUMMARY}>
        <HugeiconsIcon
          icon={
            running
              ? Loading03Icon
              : isError
                ? Cancel01Icon
                : ACTION_ICONS[toolAction(name)]
          }
          strokeWidth={2}
          aria-label={
            running ? m.tool_running() : isError ? m.tool_failed() : undefined
          }
          className={cn(
            'size-4 shrink-0',
            running && 'animate-spin',
            isError && 'text-destructive'
          )}
        />
        <span className='text-foreground shrink-0 font-medium'>{name}</span>
        <span className='min-w-0 truncate font-mono text-xs'>{target}</span>
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
          {output ? (
            <pre
              className={cn(
                'text-muted-foreground max-h-80 overflow-auto px-3 py-2 whitespace-pre-wrap',
                target && 'border-border/60 border-t'
              )}
            >
              {output}
            </pre>
          ) : null}
        </div>
      ) : null}
    </details>
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
