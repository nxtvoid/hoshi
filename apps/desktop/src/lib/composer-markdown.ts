import type { JSONContent } from '@tiptap/core'

/** The composer's chips: what they stand for, and how the prompt writes them. */
export type MentionKind = 'file' | 'pullRequest' | 'skill' | 'image'

/** An image chip in the draft's text, until sending numbers it. Its value is the image's key. */
const IMAGE_CHIP = /\[image:([0-9a-f-]+)\]/g

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
  }
}

/** The keys of the images the prompt's chips show, in order, each once. */
export function imageChipKeys(text: string): string[] {
  return [
    ...new Set(Array.from(text.matchAll(IMAGE_CHIP), (match) => match[1] ?? ''))
  ]
}

/**
 * The prompt as sent: each image chip becomes `[Image #n]`, numbered as its image is among
 * those sent, the way Claude Code names pasted images. A chip whose image is gone goes too.
 */
export function numberImageChips(
  text: string,
  numbers: ReadonlyMap<string, number>
): string {
  return text
    .replace(IMAGE_CHIP, (_, key: string) => {
      const number = numbers.get(key)
      return number === undefined ? '' : `[Image #${number}]`
    })
    .replace(/[ \t]{2,}/g, ' ')
    .trim()
}

/** A saved draft without its image chips: the images don't outlive the composer. */
export function withoutImageChips(doc: JSONContent): JSONContent {
  const isImageChip = (node: JSONContent) =>
    node.type === 'mention' && node.attrs?.kind === 'image'
  return {
    ...doc,
    content: doc.content
      ?.filter((node) => !isImageChip(node))
      .map((node) => (node.content ? withoutImageChips(node) : node))
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
