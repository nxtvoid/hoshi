import { m } from '@/paraglide/messages'
import { memo, useLayoutEffect, useRef } from 'react'
import type { ThreadItem } from '@/bindings/ThreadItem'
import type { TurnFailure } from '@/bindings/TurnFailure'
import { useTranscriptStore } from '@/state/transcript-store'

/** Within this distance of the bottom, new content keeps the view pinned to it. */
const STICK_TO_BOTTOM_PX = 80

export function Transcript({ working }: { working: boolean }) {
  const items = useTranscriptStore((state) => state.items)
  const streaming = useTranscriptStore((state) => state.streaming)
  const scrollRef = useRef<HTMLDivElement>(null)
  const pinned = useRef(true)

  // Scroll position is DOM state: follow new content only if the reader was at the bottom.
  useLayoutEffect(() => {
    const element = scrollRef.current
    if (element && pinned.current) element.scrollTop = element.scrollHeight
  })

  const streamingEntries = Object.entries(streaming).filter(
    ([itemId]) => !items.some((item) => item.id === itemId)
  )

  return (
    <div
      ref={scrollRef}
      onScroll={(event) => {
        const element = event.currentTarget
        pinned.current =
          element.scrollHeight - element.scrollTop - element.clientHeight <
          STICK_TO_BOTTOM_PX
      }}
      className='min-h-0 flex-1 overflow-y-auto'
    >
      <ol className='mx-auto flex w-full max-w-3xl flex-col gap-4 px-4 py-6'>
        {items.map((item) => (
          <li key={item.id} className='[content-visibility:auto]'>
            <Item item={item} />
          </li>
        ))}
        {streamingEntries.map(([itemId, text]) => (
          <li key={itemId}>
            <AssistantText text={text} />
          </li>
        ))}
        {working && streamingEntries.length === 0 ? (
          <li className='text-muted-foreground text-sm'>{m.working()}</li>
        ) : null}
      </ol>
    </div>
  )
}

const Item = memo(function Item({ item }: { item: ThreadItem }) {
  const { content } = item
  switch (content.kind) {
    case 'userMessage':
      return (
        <div className='flex justify-end'>
          <p className='bg-muted max-w-[85%] rounded-xl px-3 py-2 text-sm whitespace-pre-wrap'>
            {content.text}
          </p>
        </div>
      )
    case 'assistantMessage':
      return <AssistantText text={content.text} />
    case 'toolCall':
      return (
        <ToolCall
          name={content.name}
          input={content.input}
          output={content.output}
          isError={content.isError}
        />
      )
    case 'turnFailed':
      return (
        <p className='text-destructive text-sm'>
          {failureMessage(content.reason)}
        </p>
      )
  }
})

function AssistantText({ text }: { text: string }) {
  return <p className='text-sm leading-relaxed whitespace-pre-wrap'>{text}</p>
}

function ToolCall({
  name,
  input,
  output,
  isError
}: {
  name: string
  input: string
  output: string | null
  isError: boolean
}) {
  const status =
    output === null ? m.tool_running() : isError ? m.tool_failed() : null
  return (
    <details className='border-border/60 bg-card/40 group rounded-lg border text-xs'>
      <summary className='text-muted-foreground hover:text-foreground flex cursor-default items-center gap-2 px-3 py-2 select-none'>
        <span className='text-foreground font-medium'>{name}</span>
        <span className='min-w-0 flex-1 truncate font-mono'>
          {summarizeInput(input)}
        </span>
        {status ? (
          <span className={isError ? 'text-destructive' : undefined}>
            {status}
          </span>
        ) : null}
      </summary>
      {output ? (
        <pre className='border-border/60 text-muted-foreground max-h-80 overflow-auto border-t px-3 py-2 font-mono whitespace-pre-wrap'>
          {output}
        </pre>
      ) : null}
    </details>
  )
}

const SUMMARY_KEYS = ['command', 'file_path', 'path', 'pattern', 'url', 'query']

/** The one argument that says what a tool call did, such as the file or command. */
function summarizeInput(input: string) {
  try {
    const value = JSON.parse(input) as Record<string, unknown>
    for (const key of SUMMARY_KEYS) {
      if (typeof value[key] === 'string') return value[key]
    }
  } catch {
    // Truncated or non-JSON input is shown as is.
  }
  return input
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
