import type { ReactNode } from 'react'
import { cn } from '@hoshi/utils'
import { useLayoutEffect, useRef } from 'react'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import type { ComposerTrigger } from '@/lib/composer-trigger'
import { FileIcon } from '@/components/files/FileIcon'
import { PullRequestIcon } from '@/components/pull-request/PullRequestIcon'
import type { ComposerAction, Suggestion } from './use-composer-suggestions'
import {
  AiBrain01Icon,
  PencilEdit02Icon,
  SlashIcon,
  SparklesIcon,
  TaskEdit01Icon
} from '@hugeicons/core-free-icons'

const ACTION_ICONS: Record<ComposerAction, typeof SlashIcon> = {
  model: AiBrain01Icon,
  plan: TaskEdit01Icon,
  default: PencilEdit02Icon
}

const LABELS: Record<ComposerTrigger['kind'], () => string> = {
  path: m.composer_suggestions_files,
  command: m.composer_suggestions_commands,
  pullRequest: m.composer_suggestions_pull_requests,
  skill: m.composer_suggestions_skills
}

export const suggestionOptionId = (listId: string, index: number) =>
  `${listId}-option-${index}`

type ComposerSuggestionsProps = {
  listId: string
  kind: ComposerTrigger['kind']
  items: Suggestion[]
  activeId: string | null
  /** Shown below the list, or alone when it is empty. */
  status: string | null
  onHighlight: (id: string) => void
  onPick: (item: Suggestion) => void
}

/** The `@`, `/`, `#` and `$` menu floating above the composer. */
export function ComposerSuggestions({
  listId,
  kind,
  items,
  activeId,
  status,
  onHighlight,
  onPick
}: ComposerSuggestionsProps) {
  const listRef = useRef<HTMLDivElement>(null)
  const label = LABELS[kind]()

  useLayoutEffect(() => {
    if (!activeId) return
    listRef.current
      ?.querySelector(`[data-suggestion-id="${CSS.escape(activeId)}"]`)
      ?.scrollIntoView({ block: 'nearest' })
  }, [activeId])

  return (
    <div className='border-border bg-popover text-popover-foreground absolute inset-x-0 bottom-full z-20 mb-2 overflow-hidden rounded-xl border shadow-lg'>
      <div className='text-muted-foreground px-3 pt-2 pb-1 text-xs font-medium'>
        {label}
      </div>
      {items.length > 0 ? (
        <div
          ref={listRef}
          id={listId}
          role='listbox'
          aria-label={label}
          className='max-h-72 overflow-y-auto px-1 pb-1'
        >
          {items.map((item, index) => (
            <SuggestionRow
              key={item.id}
              optionId={suggestionOptionId(listId, index)}
              item={item}
              active={item.id === activeId}
              onHighlight={onHighlight}
              onPick={onPick}
            />
          ))}
        </div>
      ) : null}
      {status ? (
        <p className='text-muted-foreground px-3 pt-1 pb-2.5 text-xs'>
          {status}
        </p>
      ) : null}
    </div>
  )
}

type SuggestionRowProps = {
  optionId: string
  item: Suggestion
  active: boolean
  onHighlight: (id: string) => void
  onPick: (item: Suggestion) => void
}

function SuggestionRow({
  optionId,
  item,
  active,
  onHighlight,
  onPick
}: SuggestionRowProps) {
  const row = describe(item)

  return (
    <div
      id={optionId}
      role='option'
      tabIndex={-1}
      aria-selected={active}
      data-suggestion-id={item.id}
      className={cn(
        'flex cursor-default items-center gap-2 rounded-lg px-2 py-1.5 text-sm',
        active && 'bg-accent text-accent-foreground'
      )}
      onMouseMove={() => {
        if (!active) onHighlight(item.id)
      }}
      onMouseDown={(event) => event.preventDefault()}
      onClick={() => onPick(item)}
      onKeyDown={(event) => {
        if (event.key === 'Enter') onPick(item)
      }}
    >
      {row.icon}
      <span className='max-w-1/2 shrink-0 truncate font-medium'>
        {row.label}
      </span>
      {row.hint ? (
        <span className='text-muted-foreground max-w-1/3 shrink-0 truncate font-mono text-xs'>
          {row.hint}
        </span>
      ) : null}
      <span className='text-muted-foreground min-w-0 flex-1 truncate text-xs'>
        {row.description}
      </span>
    </div>
  )
}

const iconOf = (icon: typeof SlashIcon) => (
  <HugeiconsIcon
    icon={icon}
    strokeWidth={2}
    className='text-muted-foreground size-4 shrink-0'
  />
)

function describe(item: Suggestion): {
  icon: ReactNode
  label: string
  hint: string | null
  description: string
} {
  switch (item.type) {
    case 'path': {
      const { path, kind } = item.entry
      const slash = path.lastIndexOf('/')
      const directory = kind === 'directory'
      return {
        icon: <FileIcon path={path} directory={directory} />,
        label: `${path.slice(slash + 1)}${directory ? '/' : ''}`,
        hint: null,
        description: slash > 0 ? path.slice(0, slash) : ''
      }
    }
    case 'pullRequest': {
      const { number, title, headBranch, state } = item.pullRequest
      return {
        icon: <PullRequestIcon state={state} className='size-4' />,
        label: `#${number}`,
        hint: headBranch,
        description: title
      }
    }
    case 'skill':
      return {
        icon: iconOf(SparklesIcon),
        label: `$${item.command.name}`,
        hint: null,
        description: item.command.description
      }
    case 'command':
      return {
        icon: iconOf(item.action ? ACTION_ICONS[item.action] : SlashIcon),
        label: `/${item.command.name}`,
        hint: item.command.argumentHint,
        description: item.command.description
      }
  }
}
