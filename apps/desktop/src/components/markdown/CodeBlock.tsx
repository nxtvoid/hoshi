import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { useThemeStore } from '@/state/theme-store'
import { languageFileName } from '@/lib/file-icons'
import { useEffect, useRef, useState } from 'react'
import { codeLanguageLabel } from '@/lib/code-languages'
import { FileIcon } from '@/components/files/FileIcon'
import { Copy01Icon } from '@hugeicons/core-free-icons'
import { CopyButton } from '@hoshi/ui/components/copy-button'
import {
  cachedHighlight,
  forgetHighlight,
  highlight,
  rememberHighlight
} from '@/lib/highlight'

const PRE =
  'overflow-x-auto px-3 pb-3 font-mono text-(length:--code-font-size) leading-relaxed'

const HIGHLIGHTED =
  '[&_pre]:overflow-x-auto [&_pre]:px-3 [&_pre]:pb-3 [&_pre]:font-mono [&_pre]:text-(length:--code-font-size) [&_pre]:leading-relaxed [&_pre]:outline-none [&_span]:text-(--shiki-light) dark:[&_span]:text-(--shiki-dark)'

/** A fenced code block: its language, a copy button and the code with syntax colors. */
export function CodeBlock({
  code,
  language
}: {
  code: string
  language: string | null
}) {
  const html = useHighlighted(code, language)
  const wrap = useThemeStore((state) => state.wordWrap)
  return (
    <div className='border-border/70 bg-secondary/60 dark:bg-input/30 my-2.5 overflow-hidden rounded-lg border dark:border-transparent'>
      <div className='text-muted-foreground flex h-7 items-center justify-between gap-2 ps-3 pe-1.5 select-none'>
        <span className='flex min-w-0 items-center gap-1.5'>
          <FileIcon
            path={languageFileName(language ?? '')}
            className='size-3.5'
          />
          <span className='truncate text-[11px]'>
            {language ? codeLanguageLabel(language) : m.code_language_plain()}
          </span>
        </span>
        <CopyButton
          value={code}
          label={m.copy_code()}
          copiedLabel={m.copied()}
          icon={Copy01Icon}
          side='top'
          iconClassName='size-3'
          className='hover:text-foreground hover:bg-muted rounded-md p-1'
        />
      </div>
      {html ? (
        <div
          className={cn(
            HIGHLIGHTED,
            wrap && '[&_pre]:wrap-break-word [&_pre]:whitespace-pre-wrap'
          )}
          dangerouslySetInnerHTML={{ __html: html }}
        />
      ) : (
        <pre className={cn(PRE, wrap && 'wrap-break-word whitespace-pre-wrap')}>
          <code>{code}</code>
        </pre>
      )}
    </div>
  )
}

/** Code that changes this soon after its last change is still being written. */
const STREAMING_MS = 150

/**
 * While the code streams in, it is colored at most once every `STREAMING_MS` and the last
 * coloring stays up until the next one is ready. A block keeps only its latest coloring in
 * the cache, so the versions it went through while being written don't fill it.
 */
function useHighlighted(code: string, language: string | null) {
  const [html, setHtml] = useState(() =>
    language ? cachedHighlight(code, language) : undefined
  )
  const mounted = useRef(true)
  const changedAt = useRef<number | null>(null)
  const startedAt = useRef(Number.NEGATIVE_INFINITY)
  const started = useRef(0)
  const shown = useRef(0)
  const kept = useRef<{ code: string; language: string } | null>(null)

  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
    }
  }, [])

  useEffect(() => {
    if (!language) return
    const show = (run: number, result: string | null) => {
      const previous = kept.current
      if (
        previous &&
        (previous.code !== code || previous.language !== language)
      ) {
        forgetHighlight(previous.code, previous.language)
      }
      kept.current = result === null ? null : { code, language }
      if (result !== null) rememberHighlight(code, language, result)
      shown.current = run
      setHtml(result ?? undefined)
    }
    const now = performance.now()
    const streaming =
      changedAt.current !== null && now - changedAt.current < STREAMING_MS
    changedAt.current = now
    const cached = cachedHighlight(code, language)
    if (cached) {
      show(++started.current, cached)
      return
    }
    const start = () => {
      startedAt.current = performance.now()
      const run = ++started.current
      void highlight(code, language).then((result) => {
        if (mounted.current && run > shown.current) show(run, result)
      })
    }
    const wait = streaming
      ? Math.max(0, startedAt.current + STREAMING_MS - now)
      : 0
    if (wait === 0) {
      start()
      return
    }
    const timer = setTimeout(start, wait)
    return () => clearTimeout(timer)
  }, [code, language])

  return language ? html : undefined
}
