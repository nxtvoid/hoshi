import { cn, offsets } from '@hoshi/utils'
import { type ComponentProps, memo, useMemo } from 'react'
import { useThemeStore } from '@/state/theme-store'
import { CodeBlock } from './CodeBlock'
import { MarkdownLink } from './MarkdownLink'
import { MarkdownImage } from './MarkdownImage'
import { ImageMention } from './ImageMention'
import { IMAGE_MENTION_TAG, rehypeMentions } from './rehype-mentions'
import { rehypeLocalPaths } from './rehype-local-paths'
import type { KnownMentions } from '@/lib/prompt-segments'
import { splitMarkdownBlocks } from '@/lib/markdown-blocks'
import rehypeSanitize, { defaultSchema } from 'rehype-sanitize'
import ReactMarkdown, {
  type Components,
  defaultUrlTransform,
  type ExtraProps,
  type Options
} from 'react-markdown'
import remarkGfm from 'remark-gfm'
import rehypeRaw from 'rehype-raw'
import remarkBreaks from 'remark-breaks'

type HastElement = NonNullable<ExtraProps['node']>
const TRAILING_NEWLINE = /\r?\n$/

const COMPONENTS: Components = {
  h1: ({ node: _, ...props }) => (
    <h1 className='mt-5 mb-2 text-xl leading-snug font-semibold' {...props} />
  ),
  h2: ({ node: _, ...props }) => (
    <h2 className='mt-5 mb-2 text-lg leading-snug font-semibold' {...props} />
  ),
  h3: ({ node: _, ...props }) => (
    <h3 className='mt-5 mb-2 text-base leading-snug font-semibold' {...props} />
  ),
  h4: ({ node: _, ...props }) => (
    <h4 className='mt-4 mb-1.5 text-sm font-semibold' {...props} />
  ),
  p: ({ node: _, ...props }) => <p className='my-2.5' {...props} />,
  strong: ({ node: _, ...props }) => (
    <strong className='font-semibold' {...props} />
  ),
  ul: ({ node: _, ...props }) => (
    <ul
      className='my-2.5 list-disc ps-5 [&_ul]:my-1 [&_ul]:list-[circle] [&_ul_ul]:list-[square]'
      {...props}
    />
  ),
  ol: ({ node: _, ...props }) => (
    <ol
      className='my-2.5 list-[tight-decimal] ps-5 marker:tabular-nums [&_ol]:my-1 [&_ol]:list-[tight-lower-alpha] [&>li]:ps-1.5'
      {...props}
    />
  ),
  li: ({ node: _, className, ...props }) => (
    <li
      className={cn('[&+li]:mt-1 [&.task-list-item]:list-none', className)}
      {...props}
    />
  ),
  blockquote: ({ node: _, ...props }) => (
    <blockquote
      className='border-foreground/25 text-muted-foreground my-2.5 border-s-2 ps-3'
      {...props}
    />
  ),
  hr: () => <hr className='border-border my-4' />,
  a: ({ href, children }) => (
    <MarkdownLink href={href}>{children}</MarkdownLink>
  ),
  code: ({ node: _, className, ...props }) => (
    <code
      className={cn(
        'bg-foreground/10 rounded [box-decoration-break:clone] px-[0.35em] py-[0.1em] font-mono text-[0.85em]',
        className
      )}
      {...props}
    />
  ),
  pre: ({ node, ...props }) => {
    const block = codeBlockOf(node)
    if (block) return <CodeBlock {...block} />
    return <PlainPre {...props} />
  },
  table: ({ node: _, ...props }) => (
    <div className='my-2.5 overflow-x-auto'>
      <table className='w-full border-collapse text-xs' {...props} />
    </div>
  ),
  th: ({ node: _, ...props }) => (
    <th
      className='border-border border-b px-2 py-1.5 text-left font-semibold'
      {...props}
    />
  ),
  td: ({ node: _, ...props }) => (
    <td
      className='border-border/60 border-b px-2 py-1.5 align-top'
      {...props}
    />
  ),
  img: ({ node: _, ...props }) => <MarkdownImage {...props} />,
  input: ({ node: _, ...props }) => (
    <input className='me-1.5 align-middle' disabled {...props} />
  ),
  ...({ [IMAGE_MENTION_TAG]: ImageMention } as Components)
}

/** The code and language of a fenced block; `null` for a `pre` written as raw HTML. */
function codeBlockOf(pre: HastElement | undefined) {
  const code = pre?.children.find(
    (child): child is HastElement =>
      child.type === 'element' && child.tagName === 'code'
  )
  if (!code) return null
  const classes = code.properties.className
  const language = Array.isArray(classes)
    ? classes
        .map(String)
        .find((name) => name.startsWith('language-'))
        ?.slice('language-'.length)
        .toLowerCase()
    : undefined
  return {
    code: textOf(code).replace(TRAILING_NEWLINE, ''),
    language: language ?? null
  }
}

function textOf(node: HastElement['children'][number]): string {
  if (node.type === 'text') return node.value
  if (node.type === 'element') return node.children.map(textOf).join('')
  return ''
}

/** GitHub's own rules for HTML inside markdown, plus the sizing bots use on images. */
type PluggableList = NonNullable<Options['rehypePlugins']>

const SCHEMA = {
  ...defaultSchema,
  attributes: {
    ...defaultSchema.attributes,
    img: [...(defaultSchema.attributes?.img ?? []), 'align', 'width', 'height']
  }
}

/** An agent's answer may also show and link files on this machine, by their path. */
const AGENT_SCHEMA = {
  ...SCHEMA,
  protocols: {
    ...defaultSchema.protocols,
    href: [...(defaultSchema.protocols?.href ?? []), 'file'],
    src: [...(defaultSchema.protocols?.src ?? []), 'file']
  }
}

/** Raw HTML is allowed, as on GitHub, and then cleaned of scripts and handlers. */
const REHYPE_PLUGINS: Record<MarkdownSource, PluggableList> = {
  agent: [rehypeRaw, rehypeLocalPaths, [rehypeSanitize, AGENT_SCHEMA]],
  forge: [rehypeRaw, [rehypeSanitize, SCHEMA]]
}

const urlTransform = (url: string) =>
  url.startsWith('file:') ? url : defaultUrlTransform(url)

const REMARK_PLUGINS: Record<MarkdownSource, PluggableList> = {
  agent: [remarkGfm],
  // Comments on a forge keep their single line breaks, as the forge shows them.
  forge: [remarkGfm, remarkBreaks]
}

type MarkdownSource = 'agent' | 'forge'

/** GitHub-flavoured markdown from agents and forges, styled with the theme. */
export const Markdown = memo(function Markdown({
  text,
  source = 'agent',
  mentions,
  streaming = false,
  className
}: {
  text: string
  source?: MarkdownSource
  /** Shows these and any `@file` as chips, as the composer did. */
  mentions?: KnownMentions
  /** The text still grows at its end, so only its last block parses again. */
  streaming?: boolean
  className?: string
}) {
  const rehypePlugins = useMemo<PluggableList>(
    () =>
      mentions
        ? [...REHYPE_PLUGINS[source], [rehypeMentions, mentions]]
        : REHYPE_PLUGINS[source],
    [mentions, source]
  )
  const blocks = (streaming && splitMarkdownBlocks(text)) || [text]
  const starts = offsets(blocks.map((block) => block.length))
  return (
    <div
      className={cn(
        'text-sm leading-relaxed wrap-break-word *:first:mt-0 *:last:mb-0',
        className
      )}
    >
      {blocks.map((block, index) => {
        return (
          <MarkdownBlock
            key={starts[index]}
            text={block}
            remarkPlugins={REMARK_PLUGINS[source]}
            rehypePlugins={rehypePlugins}
          />
        )
      })}
    </div>
  )
})

const MarkdownBlock = memo(function MarkdownBlock({
  text,
  remarkPlugins,
  rehypePlugins
}: {
  text: string
  remarkPlugins: PluggableList
  rehypePlugins: PluggableList
}) {
  return (
    <ReactMarkdown
      remarkPlugins={remarkPlugins}
      rehypePlugins={rehypePlugins}
      components={COMPONENTS}
      urlTransform={urlTransform}
    >
      {text}
    </ReactMarkdown>
  )
})

function PlainPre(props: ComponentProps<'pre'>) {
  const wrap = useThemeStore((state) => state.wordWrap)
  return (
    <pre
      className={cn(
        'bg-muted my-2.5 overflow-x-auto rounded-lg p-3 font-mono text-(length:--code-font-size)',
        wrap && 'wrap-break-word whitespace-pre-wrap'
      )}
      {...props}
    />
  )
}
