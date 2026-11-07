import { useState } from 'react'
import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { Search01Icon } from '@hugeicons/core-free-icons'
import { Link, useNavigate } from '@tanstack/react-router'
import {
  NAV_GROUPS,
  SECTION_META,
  type SettingsSection,
  searchSettings
} from './sections'

const ITEM_CLASS =
  'text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:ring-ring aria-[current=page]:bg-muted aria-[current=page]:text-foreground flex items-center gap-2 rounded-lg px-2 py-1.5 text-sm outline-none focus-visible:ring-2'

export function SettingsNav({ section }: { section: SettingsSection }) {
  const navigate = useNavigate()
  const [query, setQuery] = useState('')
  const results = searchSettings(query)
  const searching = query.trim() !== ''

  const open = (target: SettingsSection) => {
    setQuery('')
    navigate({ to: '/settings/$section', params: { section: target } })
  }

  return (
    <nav
      aria-label={m.settings()}
      className='border-border bg-sidebar flex w-56 shrink-0 flex-col gap-3 border-r p-2'
    >
      <label className='text-muted-foreground hover:bg-muted/60 focus-within:bg-muted flex h-8 items-center gap-2 rounded-lg px-2 text-sm'>
        <HugeiconsIcon
          icon={Search01Icon}
          strokeWidth={2}
          className='size-4 shrink-0'
        />
        <input
          type='search'
          value={query}
          onChange={(event) => setQuery(event.currentTarget.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter' && results[0]) open(results[0].section)
            if (event.key === 'Escape' && searching) {
              event.preventDefault()
              event.stopPropagation()
              setQuery('')
            }
          }}
          placeholder={m.search_settings()}
          aria-label={m.search_settings()}
          className='placeholder:text-muted-foreground/70 text-foreground min-w-0 flex-1 bg-transparent outline-none'
        />
      </label>
      {searching ? (
        results.length === 0 ? (
          <p className='text-muted-foreground px-2 py-6 text-center text-xs'>
            {m.no_matching_settings()}
          </p>
        ) : (
          <ul className='flex flex-col gap-0.5'>
            {results.map((result) => (
              <li key={`${result.section}:${result.title}`}>
                <button
                  type='button'
                  onClick={() => open(result.section)}
                  className={cn(
                    ITEM_CLASS,
                    'w-full flex-col items-start gap-0'
                  )}
                >
                  <span className='text-foreground'>{result.title}</span>
                  <span className='text-xs'>
                    {SECTION_META[result.section].label()}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )
      ) : (
        <div className='flex flex-col gap-4'>
          {NAV_GROUPS.map((group) => (
            <div
              key={group.sections.join(' ')}
              className='flex flex-col gap-0.5'
            >
              {group.heading ? (
                <p className='text-muted-foreground px-2 pb-1 text-xs font-medium'>
                  {group.heading()}
                </p>
              ) : null}
              {group.sections.map((id) => (
                <Link
                  key={id}
                  to='/settings/$section'
                  params={{ section: id }}
                  aria-current={id === section ? 'page' : undefined}
                  className={ITEM_CLASS}
                >
                  <HugeiconsIcon
                    icon={SECTION_META[id].icon}
                    strokeWidth={2}
                    className='size-4'
                  />
                  {SECTION_META[id].label()}
                </Link>
              ))}
            </div>
          ))}
        </div>
      )}
    </nav>
  )
}
