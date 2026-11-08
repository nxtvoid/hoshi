import { useRef, useState } from 'react'
import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { Button } from '@hoshi/ui/components/button'
import { languageFileName } from '@/lib/file-icons'
import { FileIcon } from '@/components/files/FileIcon'
import {
  ArrowDown01Icon,
  Search01Icon,
  Tick02Icon
} from '@hugeicons/core-free-icons'
import {
  codeLanguage,
  codeLanguageLabel,
  searchCodeLanguages
} from '@/lib/code-languages'
import {
  Popover,
  PopoverContent,
  PopoverTrigger
} from '@hoshi/ui/components/popover'

type CodeLanguagePickerProps = {
  /** As written after the fence; empty for plain text. */
  language: string
  onChange: (language: string) => void
}

/** The language of a code block in the composer, searchable. */
export function CodeLanguagePicker({
  language,
  onChange
}: CodeLanguagePickerProps) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const searchRef = useRef<HTMLInputElement>(null)
  const current = codeLanguage(language)?.id ?? language
  const label = language ? codeLanguageLabel(language) : m.code_language_plain()
  const options = [
    ...(query.trim() ? [] : [{ id: '', label: m.code_language_plain() }]),
    ...searchCodeLanguages(query)
  ]

  const pick = (id: string) => {
    setOpen(false)
    setQuery('')
    if (id !== current) onChange(id)
  }

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setOpen(next)
        if (!next) setQuery('')
      }}
    >
      <PopoverTrigger
        render={
          <Button
            type='button'
            variant='ghost'
            size='xs'
            aria-label={m.code_language_label({ language: label })}
            className='text-muted-foreground'
          />
        }
      >
        <FileIcon path={languageFileName(current)} className='size-3.5' />
        <span className='max-w-40 truncate'>{label}</span>
        <HugeiconsIcon icon={ArrowDown01Icon} strokeWidth={2} />
      </PopoverTrigger>
      <PopoverContent
        side='bottom'
        align='start'
        initialFocus={searchRef}
        className='w-56 gap-1 p-1'
      >
        <div className='border-border flex items-center gap-2 border-b px-2 pb-1'>
          <HugeiconsIcon
            icon={Search01Icon}
            strokeWidth={2}
            className='text-muted-foreground size-3.5 shrink-0'
          />
          <input
            ref={searchRef}
            value={query}
            onChange={(event) => setQuery(event.currentTarget.value)}
            onKeyDown={(event) => {
              const first = options[0]
              if (event.key === 'Enter' && first) {
                event.preventDefault()
                pick(first.id)
              }
            }}
            placeholder={m.code_language_search()}
            aria-label={m.code_language_search()}
            className='placeholder:text-muted-foreground h-7 min-w-0 flex-1 bg-transparent text-sm outline-none'
          />
        </div>
        <div className='max-h-64 overflow-y-auto'>
          {options.length === 0 ? (
            <p className='text-muted-foreground px-2 py-1.5 text-xs'>
              {m.code_language_none()}
            </p>
          ) : (
            options.map((option) => (
              <button
                key={option.id || 'plain'}
                type='button'
                onClick={() => pick(option.id)}
                className={cn(
                  'hover:bg-accent hover:text-accent-foreground flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm',
                  option.id === current && 'font-medium'
                )}
              >
                <FileIcon
                  path={languageFileName(option.id)}
                  className='size-3.5'
                />
                <span className='min-w-0 flex-1 truncate'>{option.label}</span>
                {option.id === current ? (
                  <HugeiconsIcon
                    icon={Tick02Icon}
                    strokeWidth={2}
                    className='size-3.5 shrink-0'
                  />
                ) : null}
              </button>
            ))
          )}
        </div>
      </PopoverContent>
    </Popover>
  )
}
