import type { ExtraProps } from 'react-markdown'
import { CHIP_CLASSES } from '@/lib/chips'
import { type KnownMentions, promptSegments } from '@/lib/prompt-segments'

type HastElement = NonNullable<ExtraProps['node']>
type HastChild = HastElement['children'][number]

/** Where an `[Image #n]` chip goes, drawn by `ImageMention`. */
export const IMAGE_MENTION_TAG = 'hoshi-image'
/** Where a `[File #n]` chip goes, drawn by `FileMention`. */
export const FILE_MENTION_TAG = 'hoshi-file'

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
        : segment.image
          ? {
              type: 'element',
              tagName: IMAGE_MENTION_TAG,
              properties: {
                dataId: segment.image.id,
                dataName: segment.image.name,
                dataMimeType: segment.image.mimeType,
                dataSize: segment.image.size
              },
              children: []
            }
          : segment.file
            ? {
                type: 'element',
                tagName: FILE_MENTION_TAG,
                properties: {
                  dataId: segment.file.id,
                  dataName: segment.file.name,
                  dataMimeType: segment.file.mimeType,
                  dataSize: segment.file.size
                },
                children: []
              }
            : {
                type: 'element',
                tagName: 'span',
                properties: {
                  className: [
                    'rounded-sm',
                    'px-0.5',
                    ...CHIP_CLASSES[segment.kind].split(' ')
                  ],
                  title: segment.label ? segment.text : undefined
                },
                children: [
                  { type: 'text', value: segment.label ?? segment.text }
                ]
              }
  )
}
