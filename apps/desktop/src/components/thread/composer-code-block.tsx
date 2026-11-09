import { Extension } from '@tiptap/core'
import { highlightTokens } from '@/lib/highlight'
import CodeBlock from '@tiptap/extension-code-block'
import { Plugin, PluginKey } from '@tiptap/pm/state'
import { CodeLanguagePicker } from './CodeLanguagePicker'
import type { Node as ProseMirrorNode } from '@tiptap/pm/model'
import { Decoration, DecorationSet } from '@tiptap/pm/view'
import {
  NodeViewContent,
  type NodeViewProps,
  NodeViewWrapper,
  ReactNodeViewRenderer
} from '@tiptap/react'

/** Longer blocks stay plain, so typing in them never stalls. */
const MAX_HIGHLIGHTED_CHARS = 20_000
const HIGHLIGHT_DELAY_MS = 120

function ComposerCodeBlockView({
  node,
  editor,
  getPos,
  updateAttributes
}: NodeViewProps) {
  const language = String(node.attrs.language ?? '')

  const changeLanguage = (next: string) => {
    updateAttributes({ language: next || null })
    const position = getPos()
    if (typeof position === 'number') {
      editor
        .chain()
        .focus(position + 1 + node.content.size)
        .run()
    }
  }

  return (
    <NodeViewWrapper className='border-border/70 bg-secondary/60 dark:bg-input/30 my-1 overflow-hidden rounded-lg border dark:border-transparent'>
      <div contentEditable={false} className='flex h-7 items-center px-1'>
        <CodeLanguagePicker language={language} onChange={changeLanguage} />
      </div>
      <pre className='overflow-x-auto px-3 pb-2 font-mono leading-relaxed text-(length:--code-font-size)'>
        <NodeViewContent<'code'>
          as='code'
          className='block min-h-[1lh] whitespace-pre-wrap'
        />
      </pre>
    </NodeViewWrapper>
  )
}

/** A fenced code block with a language picker above its code. */
export const ComposerCodeBlock = CodeBlock.extend({
  addNodeView() {
    return ReactNodeViewRenderer(ComposerCodeBlockView)
  }
})

const highlightKey = new PluginKey<DecorationSet>('codeBlockHighlight')

/** Colors the code blocks, a moment after typing stops. */
export const CodeBlockHighlight = Extension.create({
  name: 'codeBlockHighlight',
  addProseMirrorPlugins() {
    return [
      new Plugin<DecorationSet>({
        key: highlightKey,
        state: {
          init: () => DecorationSet.empty,
          apply: (transaction, decorations) =>
            (transaction.getMeta(highlightKey) as DecorationSet | undefined) ??
            decorations.map(transaction.mapping, transaction.doc)
        },
        props: {
          decorations: (state) => highlightKey.getState(state)
        },
        view: (view) => {
          let timer: ReturnType<typeof setTimeout> | undefined
          let painted: ProseMirrorNode | null = null

          const paint = async () => {
            const doc = view.state.doc
            painted = doc
            const blocks: { pos: number; code: string; language: string }[] = []
            doc.descendants((node, pos) => {
              if (node.type.name !== 'codeBlock') return true
              if (node.textContent.length <= MAX_HIGHLIGHTED_CHARS) {
                blocks.push({
                  pos,
                  code: node.textContent,
                  language: String(node.attrs.language ?? '')
                })
              }
              return false
            })
            const decorations: Decoration[] = []
            for (const block of blocks) {
              const tokens = await highlightTokens(block.code, block.language)
              for (const token of tokens ?? []) {
                decorations.push(
                  Decoration.inline(
                    block.pos + 1 + token.from,
                    block.pos + 1 + token.to,
                    { style: token.style }
                  )
                )
              }
            }
            if (view.isDestroyed || view.state.doc !== doc) return
            view.dispatch(
              view.state.tr
                .setMeta(highlightKey, DecorationSet.create(doc, decorations))
                .setMeta('addToHistory', false)
            )
          }

          const schedule = () => {
            clearTimeout(timer)
            timer = setTimeout(() => void paint(), HIGHLIGHT_DELAY_MS)
          }

          schedule()
          return {
            update: (next) => {
              if (next.state.doc !== painted) schedule()
            },
            destroy: () => clearTimeout(timer)
          }
        }
      })
    ]
  }
})
