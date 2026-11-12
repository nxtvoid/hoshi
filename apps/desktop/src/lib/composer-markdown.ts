import type { JSONContent } from '@tiptap/core'

/**
 * The composer's chips: what they stand for, and how the prompt writes them. A `file` is a
 * path of the workspace; an `attachedFile` is a file attached to the draft.
 */
export type MentionKind =
  | 'file'
  | 'pullRequest'
  | 'skill'
  | 'image'
  | 'attachedFile'

/**
 * An image or file chip in the draft's text, until sending numbers it. Its value is the
 * attachment's key.
 */
const ATTACHMENT_CHIP = /\[(?:image|file):([0-9a-f-]+)\]/g

const isAttachmentChip = (node: JSONContent) =>
  node.type === 'mention' &&
  (node.attrs?.kind === 'image' || node.attrs?.kind === 'attachedFile')

export function mentionSource(kind: MentionKind, value: string) {
  switch (kind) {
    case 'file':
      return /\s/.test(value) ? `@"${value}"` : `@${value}`
    case 'pullRequest':
      return `#${value}`
    case 'skill':
      return `$${value}`
    case 'image':
      return `[image:${value}]`
    case 'attachedFile':
      return `[file:${value}]`
  }
}

/** The keys of the images and files the prompt's chips show, in order, each once. */
export function attachmentChipKeys(text: string): string[] {
  return [
    ...new Set(
      Array.from(text.matchAll(ATTACHMENT_CHIP), (match) => match[1] ?? '')
    )
  ]
}

/**
 * The prompt as sent: each image or file chip becomes its label in brackets, such as
 * `[Image #1]` or `[File #2]`, the way Claude Code names pasted images. A chip whose
 * attachment is gone goes too.
 */
export function numberAttachmentChips(
  text: string,
  labels: ReadonlyMap<string, string>
): string {
  return text
    .replace(ATTACHMENT_CHIP, (_, key: string) => {
      const label = labels.get(key)
      return label === undefined ? '' : `[${label}]`
    })
    .replace(/[ \t]{2,}/g, ' ')
    .trim()
}

/**
 * A saved draft with the chips of the images and files it still has, by key; an image's
 * chip shows `sources`' preview of it. The chips of attachments gone are left out.
 */
export function keepAttachmentChips(
  doc: JSONContent,
  sources: ReadonlyMap<string, string>
): JSONContent {
  return {
    ...doc,
    content: doc.content
      ?.filter(
        (node) =>
          !isAttachmentChip(node) || sources.has(String(node.attrs?.value))
      )
      .map((node) => {
        if (isAttachmentChip(node) && node.attrs?.kind === 'image') {
          const src = sources.get(String(node.attrs?.value))
          return { ...node, attrs: { ...node.attrs, src } }
        }
        return node.content ? keepAttachmentChips(node, sources) : node
      })
  }
}

/** A draft to save, its image chips without their preview, which can be a whole image. */
export function withoutImageSources(doc: JSONContent): JSONContent {
  return {
    ...doc,
    content: doc.content?.map((node) => {
      if (node.type === 'mention' && node.attrs?.kind === 'image') {
        const { src: _, ...attrs } = node.attrs
        return { ...node, attrs }
      }
      return node.content ? withoutImageSources(node) : node
    })
  }
}

/** Two drafts as one, `first` above `second`; `null` stands for an empty draft. */
export function joinDocuments(
  first: JSONContent | null,
  second: JSONContent | null
): JSONContent | null {
  if (!first) return second
  if (!second) return first
  return {
    ...first,
    content: [...(first.content ?? []), ...(second.content ?? [])]
  }
}

const DELIMITERS: Record<string, string> = {
  bold: '**',
  italic: '*',
  strike: '~~',
  code: '`'
}
/** Opened first so the others never land inside a code span. */
const MARK_ORDER = ['bold', 'italic', 'strike', 'code']

/**
 * The composer's document as the markdown the agent reads. Text is written as
 * typed, never escaped: what the user sees is what is sent.
 */
export function toMarkdown(doc: JSONContent): string {
  return blocks(doc.content ?? []).trim()
}

function blocks(nodes: JSONContent[]): string {
  let out = ''
  nodes.forEach((node, index) => {
    if (index > 0) {
      const previous = nodes[index - 1]
      const plain = previous?.type === 'paragraph' && node.type === 'paragraph'
      out += plain ? '\n' : '\n\n'
    }
    out += block(node)
  })
  return out
}

function block(node: JSONContent): string {
  const children = node.content ?? []
  switch (node.type) {
    case 'paragraph':
      return inline(children)
    case 'heading':
      return `${'#'.repeat(Number(node.attrs?.level ?? 1))} ${inline(children)}`
    case 'blockquote':
      return prefixLines(blocks(children), '> ', '>')
    case 'codeBlock': {
      const language = node.attrs?.language ?? ''
      return `\`\`\`${language}\n${children.map((child) => child.text ?? '').join('')}\n\`\`\``
    }
    case 'bulletList':
      return children.map((item) => listItem(item, '- ')).join('\n')
    case 'orderedList': {
      const start = Number(node.attrs?.start ?? 1)
      return children
        .map((item, index) => listItem(item, `${start + index}. `))
        .join('\n')
    }
    case 'horizontalRule':
      return '---'
    default:
      return inline(children)
  }
}

function listItem(item: JSONContent, marker: string) {
  const body = blocks(item.content ?? [])
  const indent = ' '.repeat(marker.length)
  return body
    .split('\n')
    .map((line, index) =>
      index === 0 ? marker + line : line === '' ? '' : indent + line
    )
    .join('\n')
}

function prefixLines(text: string, prefix: string, emptyPrefix: string) {
  return text
    .split('\n')
    .map((line) => (line === '' ? emptyPrefix : prefix + line))
    .join('\n')
}

function inline(nodes: JSONContent[]): string {
  let out = ''
  let open: string[] = []
  const close = (keep: number) => {
    while (open.length > keep) out += DELIMITERS[open.pop() ?? ''] ?? ''
  }
  for (const node of nodes) {
    const marks = MARK_ORDER.filter((mark) =>
      node.marks?.some((candidate) => candidate.type === mark)
    )
    const content = leaf(node)
    if (node.type !== 'text') {
      close(0)
      out += content
      continue
    }
    let shared = 0
    while (shared < open.length && open[shared] === marks[shared]) shared++
    close(shared)
    for (const mark of marks.slice(shared)) {
      out += DELIMITERS[mark]
      open = [...open, mark]
    }
    out += content
  }
  close(0)
  return out
}

function leaf(node: JSONContent): string {
  switch (node.type) {
    case 'text':
      return node.text ?? ''
    case 'hardBreak':
      return '\n'
    case 'mention':
      return mentionSource(
        node.attrs?.kind as MentionKind,
        String(node.attrs?.value ?? '')
      )
    default:
      return ''
  }
}

/** A prompt as plain paragraphs, one per line, the way it was sent. */
export function fromText(text: string): JSONContent {
  return {
    type: 'doc',
    content: text.split('\n').map((line) => ({
      type: 'paragraph',
      content: line ? [{ type: 'text', text: line }] : []
    }))
  }
}
