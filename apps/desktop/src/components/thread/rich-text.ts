import { cn } from '@hoshi/utils'

/** How rich text reads while it is edited, in the composer and in a note: the same marks, lists, quotes and code. */
export const RICH_TEXT_CLASSES = cn(
  'wrap-break-word whitespace-pre-wrap outline-none',
  '[&_p]:my-0 [&_strong]:font-semibold',
  '[&_blockquote]:border-foreground/25 [&_blockquote]:text-muted-foreground [&_blockquote]:my-1 [&_blockquote]:border-s-2 [&_blockquote]:ps-3',
  '[&_code]:bg-foreground/10 [&_code]:rounded [&_code]:px-[0.35em] [&_code]:py-[0.1em] [&_code]:font-mono [&_code]:text-[0.85em]',
  '[&_pre_code]:bg-transparent [&_pre_code]:p-0 [&_pre_code]:text-xs [&_pre_span]:text-(--shiki-light) dark:[&_pre_span]:text-(--shiki-dark)',
  '[&_ol]:my-1 [&_ol]:list-decimal [&_ol]:ps-5 [&_ul]:my-1 [&_ul]:list-disc [&_ul]:ps-5',
  '[&_h1]:text-xl [&_h1]:font-semibold [&_h2]:text-lg [&_h2]:font-semibold [&_h3]:text-base [&_h3]:font-semibold',
  '[&_hr]:border-border [&_hr]:my-2',
  '[&_p.is-editor-empty:first-child]:before:text-muted-foreground [&_p.is-editor-empty:first-child]:before:pointer-events-none [&_p.is-editor-empty:first-child]:before:float-left [&_p.is-editor-empty:first-child]:before:h-0 [&_p.is-editor-empty:first-child]:before:content-[attr(data-placeholder)]'
)
