import { m } from '@/paraglide/messages'
import { languageFileName } from '@/lib/file-icons'
import { codeLanguageLabel } from '@/lib/code-languages'
import { FileIcon } from '@/components/files/FileIcon'
import { Copy01Icon } from '@hugeicons/core-free-icons'
import { cachedHighlight, highlight } from '@/lib/highlight'
import { CopyButton } from '@hoshi/ui/components/copy-button'
import { useDeferredValue, useEffect, useState } from 'react'

const PRE = 'overflow-x-auto px-3 pb-3 font-mono text-xs leading-relaxed'

const HIGHLIGHTED =
  '[&_pre]:overflow-x-auto [&_pre]:px-3 [&_pre]:pb-3 [&_pre]:font-mono [&_pre]:text-xs [&_pre]:leading-relaxed [&_pre]:outline-none [&_span]:text-(--shiki-light) dark:[&_span]:text-(--shiki-dark)'

/** A fenced code block: its language, a copy button and the code with syntax colors. */
export function CodeBlock({
  code,
  language
}: {
  code: string
  language: string | null
}) {
  const html = useHighlighted(code, language)
  return (
    <div className='border-border/70 bg-secondary/60 dark:bg-input/30 my-2.5 overflow-hidden rounded-lg border dark:border-transparent'>
      <div className='text-muted-foreground flex h-7 items-center justify-between gap-2 ps-3 pe-1.5 select-none'>
        <span className='flex min-w-0 items-center gap-1.5'>
          {language ? (
            <FileIcon path={languageFileName(language)} className='size-3.5' />
          ) : null}
          <span className='truncate text-[11px]'>
            {language ? codeLanguageLabel(language) : null}
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
          className={HIGHLIGHTED}
          dangerouslySetInnerHTML={{ __html: html }}
        />
      ) : (
        <pre className={PRE}>
          <code>{code}</code>
        </pre>
      )}
    </div>
  )
}

/** While the code streams in, the last coloring stays up until the next one is ready. */
function useHighlighted(code: string, language: string | null) {
  const deferred = useDeferredValue(code)
  const [html, setHtml] = useState(() =>
    language ? cachedHighlight(code, language) : undefined
  )

  useEffect(() => {
    if (!language) return
    let live = true
    void highlight(deferred, language).then((result) => {
      if (live) setHtml(result ?? undefined)
    })
    return () => {
      live = false
    }
  }, [deferred, language])

  return language ? html : undefined
}
