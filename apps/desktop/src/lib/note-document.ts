import type { JSONContent } from '@tiptap/core'
import { fenceLanguage } from '@/lib/code-languages'

const FENCE_END = '```'

const paragraph = (line: string): JSONContent => ({
  type: 'paragraph',
  content: line ? [{ type: 'text', text: line }] : []
})

const isBlank = (node: JSONContent | undefined) =>
  node?.type === 'paragraph' && (node.content ?? []).length === 0

/**
 * A note written as plain text, as its editor shows it: one paragraph per line, and what
 * sits between code fences as a code block. The blank line that sets a code block apart
 * from the text around it is the block's own spacing, not a paragraph. Nothing else is
 * read as markdown, so text that only looks like it stays as typed.
 */
export function noteDocument(text: string): JSONContent {
  const lines = text.split('\n')
  const content: JSONContent[] = []
  let index = 0
  while (index < lines.length) {
    const line = lines[index] ?? ''
    const language = fenceLanguage(line)
    const end =
      language === null
        ? -1
        : lines.findIndex(
            (candidate, at) => at > index && candidate.trim() === FENCE_END
          )
    if (language === null || end === -1) {
      content.push(paragraph(line))
      index += 1
      continue
    }
    if (content.length > 1 && isBlank(content.at(-1))) content.pop()
    const code = lines.slice(index + 1, end).join('\n')
    content.push({
      type: 'codeBlock',
      attrs: { language: language || null },
      content: code ? [{ type: 'text', text: code }] : []
    })
    index = end + 1
    if (index < lines.length - 1 && lines[index] === '') index += 1
  }
  return { type: 'doc', content }
}

/** The one stretch two texts differ in: what `before` had there, and what `after` has. */
export function changedStretch(before: string, after: string) {
  let start = 0
  const shortest = Math.min(before.length, after.length)
  while (start < shortest && before[start] === after[start]) start += 1
  let end = 0
  while (
    end < shortest - start &&
    before[before.length - 1 - end] === after[after.length - 1 - end]
  ) {
    end += 1
  }
  return {
    from: before.slice(start, before.length - end),
    to: after.slice(start, after.length - end)
  }
}

/**
 * `doc` with `from` replaced by `to` in the one run of text that holds it. `null` when no
 * single run does, or more than one, as with a value spread over several lines.
 */
export function replaceInDocument(
  doc: JSONContent,
  from: string,
  to: string
): JSONContent | null {
  if (from === '') return null
  let found = 0
  const replace = (node: JSONContent): JSONContent => {
    if (node.type === 'text' && node.text?.includes(from)) {
      found += node.text.split(from).length - 1
      return { ...node, text: node.text.replace(from, to) }
    }
    if (!node.content) return node
    return { ...node, content: node.content.map(replace) }
  }
  const replaced = replace(doc)
  return found === 1 ? replaced : null
}
