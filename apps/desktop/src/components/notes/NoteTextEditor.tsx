import { useImperativeHandle, useMemo, useRef } from 'react'
import type { Ref } from 'react'
import { cn } from '@hoshi/utils'
import StarterKit from '@tiptap/starter-kit'
import type { JSONContent } from '@tiptap/core'
import { Placeholder } from '@tiptap/extensions'
import { EditorContent, useEditor } from '@tiptap/react'
import type { Node as ProseMirrorNode } from '@tiptap/pm/model'
import { openFencedBlock } from '@/components/thread/code-fence'
import { RICH_TEXT_CLASSES } from '@/components/thread/rich-text'
import {
  CodeBlockHighlight,
  ComposerCodeBlock
} from '@/components/thread/composer-code-block'

export type NoteTextEditorHandle = {
  /** Shows `document` in place of what the editor holds, without reporting a change. */
  setDocument: (document: JSONContent) => void
}

type NoteTextEditorProps = {
  ref: Ref<NoteTextEditorHandle>
  /** What the editor opens with. */
  initialDocument: JSONContent
  placeholder: string
  /**
   * The document after each edit, as the editor holds it: turning it into what is saved
   * is left for when it is saved, so typing never pays for the whole note.
   */
  onChange: (document: ProseMirrorNode, empty: boolean) => void
  onBlur: () => void
}

/**
 * A note's text, rich as the composer's: markdown as it is typed, lists, quotes, and code
 * blocks in their language. Enter is a new line, as in a document.
 */
export function NoteTextEditor({
  ref,
  initialDocument,
  placeholder,
  onChange,
  onBlur
}: NoteTextEditorProps) {
  const current = { onChange, onBlur }
  const handlers = useRef(current)
  handlers.current = current

  const options = useMemo(
    () => ({
      extensions: [
        StarterKit.configure({
          link: false,
          underline: false,
          codeBlock: false
        }),
        ComposerCodeBlock,
        CodeBlockHighlight,
        Placeholder.configure({
          placeholder: ({ node }) =>
            node.type.name === 'paragraph' ? placeholder : ''
        })
      ],
      editorProps: {
        attributes: {
          role: 'textbox',
          'aria-label': placeholder,
          'aria-multiline': 'true',
          class: cn(
            RICH_TEXT_CLASSES,
            'min-h-full px-6 pt-5 pb-10 text-sm leading-relaxed'
          )
        },
        handleKeyDown: openFencedBlock
      }
    }),
    [placeholder]
  )

  const editor = useEditor({
    autofocus: 'end',
    content: initialDocument,
    ...options,
    onUpdate: ({ editor }) =>
      handlers.current.onChange(editor.state.doc, editor.isEmpty),
    onBlur: () => handlers.current.onBlur()
  })

  useImperativeHandle(
    ref,
    () => ({
      setDocument: (document) => {
        editor?.commands.setContent(document, { emitUpdate: false })
      }
    }),
    [editor]
  )

  return (
    <EditorContent
      editor={editor}
      className='mx-auto min-h-0 w-full max-w-3xl flex-1 overflow-y-auto'
    />
  )
}
