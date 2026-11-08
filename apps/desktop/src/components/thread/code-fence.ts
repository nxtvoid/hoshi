import { TextSelection } from '@tiptap/pm/state'
import type { EditorView } from '@tiptap/pm/view'
import { fenceLanguage } from '@/lib/code-languages'

/**
 * Enter or Shift+Enter after a fence typed alone on its line, like ```tsx,
 * opens a code block in that language.
 */
export function openFencedBlock(view: EditorView, event: KeyboardEvent) {
  if (
    event.key !== 'Enter' ||
    event.ctrlKey ||
    event.metaKey ||
    event.isComposing
  ) {
    return false
  }
  const { state } = view
  const { $from, empty } = state.selection
  const codeBlock = state.schema.nodes.codeBlock
  if (!empty || !codeBlock || $from.parent.type.name !== 'paragraph') {
    return false
  }
  const language = fenceLanguage($from.parent.textContent)
  if (language === null) return false
  const start = $from.before()
  const transaction = state.tr.replaceWith(
    start,
    $from.after(),
    codeBlock.create({ language: language || null })
  )
  view.dispatch(
    transaction.setSelection(TextSelection.create(transaction.doc, start + 1))
  )
  return true
}
