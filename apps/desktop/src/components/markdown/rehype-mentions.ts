import type { ExtraProps } from 'react-markdown'
import { CHIP_CLASSES } from '@/lib/chips'
import { type KnownMentions, promptSegments } from '@/lib/prompt-segments'

type HastElement = NonNullable<ExtraProps['node']>
type HastChild = HastElement['children'][number]

/** Mentions written inside these stay as they are. */
const LITERAL = new Set(['code', 'pre', 'a'])

/** Shows a message's `@file`, `#123` and `$name` mentions as chips. */
export function rehypeMentions(known: KnownMentions) {
  return (tree: { children: HastChild[] }) => {
    tree.children = tree.children.flatMap((child) => withChips(child, known))
  }
}

function withChips(node: HastChild, known: KnownMentions): HastChild[] {
  if (node.type === 'element') {
    if (!LITERAL.has(node.tagName)) {
      node.children = node.children.flatMap((child) => withChips(child, known))
    }
    return [node]
  }
  if (node.type !== 'text') return [node]
  const segments = promptSegments(node.value, known)
  if (
    segments.every(
      (segment) => segment.kind === 'text' || segment.kind === 'command'
    )
  ) {
    return [node]
  }
  return segments.map(
    (segment): HastChild =>
      segment.kind === 'text' || segment.kind === 'command'
        ? { type: 'text', value: segment.text }
        : {
            type: 'element',
            tagName: 'span',
            properties: {
              className: [
                'rounded-sm',
                'px-0.5',
                ...CHIP_CLASSES[segment.kind].split(' ')
              ]
            },
            children: [{ type: 'text', value: segment.text }]
          }
  )
}
