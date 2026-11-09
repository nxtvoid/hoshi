import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { HugeiconsIcon } from '@hugeicons/react'
import { Input } from '@hoshi/ui/components/input'
import type { PullRequestLabel } from '@/bindings/PullRequestLabel'
import type { PullRequestActions } from './use-pull-request-actions'
import {
  Loading03Icon,
  Tag01Icon,
  Tick02Icon,
  UserAdd01Icon
} from '@hugeicons/core-free-icons'
import {
  Popover,
  PopoverContent,
  PopoverTrigger
} from '@hoshi/ui/components/popover'

const SEARCH_DELAY_MS = 250

const LIST =
  'flex max-h-60 flex-col overflow-y-auto overscroll-contain [scrollbar-gutter:stable]'

/** What a picker's list says when it has nothing to show. */
function ListNote({ loading, text }: { loading?: boolean; text: string }) {
  return (
    <li className='text-muted-foreground flex items-center gap-2 px-2 py-1.5 text-xs'>
      {loading ? (
        <HugeiconsIcon
          icon={Loading03Icon}
          strokeWidth={2}
          className='size-3.5 animate-spin'
        />
      ) : null}
      {text}
    </li>
  )
}

function EditButton({
  label,
  icon
}: {
  label: string
  icon: typeof Tag01Icon
}) {
  return (
    <PopoverTrigger
      render={
        <button
          type='button'
          aria-label={label}
          title={label}
          className='text-muted-foreground hover:text-foreground hover:bg-accent focus-visible:ring-ring flex size-6 cursor-pointer items-center justify-center rounded-md outline-none focus-visible:ring-2'
        />
      }
    >
      <HugeiconsIcon icon={icon} strokeWidth={2} className='size-3.5' />
    </PopoverTrigger>
  )
}

/** Searches the accounts that can review and asks the chosen one. */
export function ReviewerPicker({
  threadId,
  current,
  author,
  actions
}: {
  threadId: string
  current: string[]
  /** Forges refuse to ask the author for a review. */
  author: string | null
  actions: PullRequestActions
}) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [candidates, setCandidates] = useState<string[] | null>(null)
  /** The query `candidates` answered; another one is being searched while they differ. */
  const [answered, setAnswered] = useState<string | null>(null)

  useEffect(() => {
    if (!open) return
    let current = true
    const timer = setTimeout(() => {
      invoke<string[]>('get_reviewer_candidates', { threadId, query })
        .catch(() => [])
        .then((value) => {
          if (!current) return
          setCandidates(value)
          setAnswered(query)
        })
    }, SEARCH_DELAY_MS)
    return () => {
      current = false
      clearTimeout(timer)
    }
  }, [open, threadId, query])

  const searching = answered !== query
  const shown =
    candidates?.filter(
      (login) => login !== author && !current.includes(login)
    ) ?? []

  return (
    <Popover modal open={open} onOpenChange={setOpen}>
      <EditButton label={m.request_review()} icon={UserAdd01Icon} />
      <PopoverContent align='start' className='w-64 gap-1 p-1.5'>
        <Input
          autoFocus
          value={query}
          placeholder={m.search_people()}
          aria-label={m.search_people()}
          onChange={(event) => setQuery(event.currentTarget.value)}
          className='h-7 text-xs md:text-xs'
        />
        <ul className={LIST}>
          {searching && shown.length === 0 ? (
            <ListNote loading text={m.searching_people()} />
          ) : null}
          {!searching && shown.length === 0 ? (
            <ListNote
              text={query.trim() ? m.no_matches() : m.no_reviewers_available()}
            />
          ) : null}
          {shown.map((login) => (
            <li key={login}>
              <button
                type='button'
                disabled={actions.pending}
                onClick={async () => {
                  const done = await actions.run({
                    type: 'requestReviewers',
                    names: [login]
                  })
                  if (done) setOpen(false)
                }}
                className='hover:bg-accent focus-visible:ring-ring w-full rounded-md px-2 py-1.5 text-left text-xs outline-none focus-visible:ring-2'
              >
                {login}
              </button>
            </li>
          ))}
        </ul>
      </PopoverContent>
    </Popover>
  )
}

/** Every label of the repository, toggled on the pull request with a click. */
export function LabelPicker({
  threadId,
  current,
  actions
}: {
  threadId: string
  current: string[]
  actions: PullRequestActions
}) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [labels, setLabels] = useState<PullRequestLabel[] | null>(null)

  useEffect(() => {
    if (!open || labels !== null) return
    let active = true
    invoke<PullRequestLabel[]>('get_repository_labels', { threadId })
      .then((value) => active && setLabels(value))
      .catch(() => active && setLabels([]))
    return () => {
      active = false
    }
  }, [open, threadId, labels])

  const needle = query.trim().toLowerCase()
  const shown =
    labels?.filter((label) => label.name.toLowerCase().includes(needle)) ?? []

  return (
    <Popover modal open={open} onOpenChange={setOpen}>
      <EditButton label={m.edit_labels()} icon={Tag01Icon} />
      <PopoverContent align='start' className='w-64 gap-1 p-1.5'>
        <Input
          autoFocus
          value={query}
          placeholder={m.filter_labels()}
          aria-label={m.filter_labels()}
          onChange={(event) => setQuery(event.currentTarget.value)}
          className='h-7 text-xs md:text-xs'
        />
        <ul className={LIST}>
          {labels === null ? (
            <ListNote loading text={m.loading_labels()} />
          ) : null}
          {labels !== null && shown.length === 0 ? (
            <ListNote
              text={
                labels.length === 0 ? m.no_labels_available() : m.no_matches()
              }
            />
          ) : null}
          {shown.map((label) => {
            const applied = current.includes(label.name)
            return (
              <li key={label.name}>
                <button
                  type='button'
                  aria-pressed={applied}
                  disabled={actions.pending}
                  onClick={() =>
                    void actions.run(
                      applied
                        ? { type: 'removeLabel', name: label.name }
                        : { type: 'addLabels', names: [label.name] }
                    )
                  }
                  className='hover:bg-accent focus-visible:ring-ring flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs outline-none focus-visible:ring-2'
                >
                  <span
                    className='size-2.5 shrink-0 rounded-full'
                    style={{ backgroundColor: `#${label.color}` }}
                  />
                  <span className='min-w-0 flex-1 truncate'>{label.name}</span>
                  <HugeiconsIcon
                    icon={Tick02Icon}
                    strokeWidth={2}
                    className={cn('size-3.5', !applied && 'invisible')}
                  />
                </button>
              </li>
            )
          })}
        </ul>
      </PopoverContent>
    </Popover>
  )
}
