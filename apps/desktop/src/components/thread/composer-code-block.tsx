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
import {
  type BlockTokens,
  type CodeBlockAt,
  codeBlocks,
  highlightedRuns,
  mapCodeBlocks,
  sameCodeBlocks
} from './code-block-highlight'

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

type HighlightState = { decorations: DecorationSet; blocks: CodeBlockAt[] }

const highlightKey = new PluginKey<HighlightState>('codeBlockHighlight')

/** Colors the code blocks, a moment after typing stops. */
export const CodeBlockHighlight = Extension.create({
  name: 'codeBlockHighlight',
  addProseMirrorPlugins() {
    return [
      new Plugin<HighlightState>({
        key: highlightKey,
        state: {
          init: () => ({ decorations: DecorationSet.empty, blocks: [] }),
          apply: (transaction, current) =>
            (transaction.getMeta(highlightKey) as HighlightState | undefined) ??
            (transaction.docChanged
              ? {
                  decorations: current.decorations.map(
                    transaction.mapping,
                    transaction.doc
                  ),
                  blocks: mapCodeBlocks(current.blocks, transaction.mapping)
                }
              : current)
        },
        props: {
          decorations: (state) => highlightKey.getState(state)?.decorations
        },
        view: (view) => {
          let timer: ReturnType<typeof setTimeout> | undefined
          let painted: ProseMirrorNode | null = null
          const cache: BlockTokens = new WeakMap()

          const paint = async () => {
            const doc = view.state.doc
            painted = doc
            const blocks = codeBlocks(doc)
            const shown = highlightKey.getState(view.state)?.blocks ?? []
            if (sameCodeBlocks(blocks, shown)) return
            const runs = await highlightedRuns(blocks, cache, highlightTokens)
            if (view.isDestroyed || view.state.doc !== doc) return
            const decorations = DecorationSet.create(
              doc,
              runs.map((run) =>
                Decoration.inline(run.from, run.to, { style: run.style })
              )
            )
            view.dispatch(
              view.state.tr
                .setMeta(highlightKey, { decorations, blocks })
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
