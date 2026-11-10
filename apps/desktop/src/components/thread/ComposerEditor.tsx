import { useImperativeHandle, useRef } from 'react'
import type { Ref } from 'react'
import { cn } from '@hoshi/utils'
import { type PastedImages, pastedImages } from '@/lib/pasted-images'
import { CHIP_CLASSES } from '@/lib/chips'
import { fileIcon } from '@/lib/file-icons'
import StarterKit from '@tiptap/starter-kit'
import { Placeholder } from '@tiptap/extensions'
import type { EditorState } from '@tiptap/pm/state'
import { Plugin, PluginKey } from '@tiptap/pm/state'
import { openFencedBlock } from './code-fence'
import { CodeBlockHighlight, ComposerCodeBlock } from './composer-code-block'
import type { EditorView } from '@tiptap/pm/view'
import { EditorContent, useEditor } from '@tiptap/react'
import { Decoration, DecorationSet } from '@tiptap/pm/view'
import type { Node as ProseMirrorNode } from '@tiptap/pm/model'
import type { MessageAttachment } from '@/bindings/MessageAttachment'
import { type ComposerTrigger, detectTrigger } from '@/lib/composer-trigger'
import { Extension, type JSONContent, Node } from '@tiptap/core'
import {
  fromText,
  type MentionKind,
  mentionSource,
  toMarkdown
} from '@/lib/composer-markdown'

/** A trigger with where its token sits in the document. */
export type LocatedTrigger = ComposerTrigger & { from: number; to: number }

export type ComposerSnapshot = {
  /** The prompt as markdown, exactly as it will be sent. */
  text: string
  empty: boolean
  trigger: LocatedTrigger | null
  focused: boolean
}

export type ComposerInsert =
  | { text: string; keepOpen?: boolean }
  | {
      mention: {
        kind: MentionKind
        value: string
        attachment: MessageAttachment | null
      }
    }
  | { remove: true }

/** Where the caret is, for keys that act at the edges of the prompt. */
export type KeyContext = {
  inCodeBlock: boolean
  onFirstLine: () => boolean
  onLastLine: () => boolean
}

export type ComposerEditorHandle = {
  focus: () => void
  setText: (text: string) => void
  /** The document as it is, chips included; `null` when empty. */
  document: () => JSONContent | null
  /** Shows `doc` as it was read, chips included. */
  setDocument: (doc: JSONContent) => void
  replace: (trigger: LocatedTrigger, insert: ComposerInsert) => void
  /** What the pull request and skill chips stand for. */
  mentions: () => MessageAttachment[]
  /** Keeps the editor's combobox state in step with the menu. */
  setMenu: (menu: { listId: string | null; activeId: string | null }) => void
  /** Pastes `text` at the caret as if the user had. */
  paste: (text: string) => void
  /** Shows images just added as chips at the caret, when the prompt has text to place them in. */
  insertImages: (images: ImageChip[]) => void
  /** Takes away the chips of an image removed from the draft. */
  removeImage: (key: string) => void
}

/** An image of the draft, as its chip in the text shows it. */
export type ImageChip = {
  key: string
  name: string
  /** Its size, as shown after the name. */
  detail: string
  previewUrl: string
}

type ComposerEditorProps = {
  ref: Ref<ComposerEditorHandle>
  placeholder: string
  label: string
  /** On the scrolling box around the text. */
  className?: string
  /** On the text itself: its padding and least height. */
  contentClassName?: string
  onChange: (snapshot: ComposerSnapshot) => void
  /** What the editor opens with. */
  initialContent?: JSONContent
  /** The document after each edit; `null` once it is empty. */
  onDocument?: (doc: JSONContent | null) => void
  onBlur?: () => void
  /** `true` when the composer handled the key. */
  onKeyDown: (event: KeyboardEvent, context: KeyContext) => boolean
  /** Images pasted; the editor leaves them to the composer. */
  onImages: (pasted: PastedImages) => void
}

const leafText = (node: ProseMirrorNode) =>
  node.type.name === 'hardBreak' ? '\n' : '￼'

/** The `@`, `/`, `#` or `$` token the caret is in, outside code. */
function locateTrigger(state: EditorState): LocatedTrigger | null {
  const { selection, doc } = state
  const { $from } = selection
  if (
    !selection.empty ||
    !$from.parent.isTextblock ||
    $from.parent.type.name === 'codeBlock' ||
    $from.marks().some((mark) => mark.type.name === 'code')
  ) {
    return null
  }
  const before = doc.textBetween(0, $from.pos, '\n', leafText)
  const after = $from.parent.textBetween(
    $from.parentOffset,
    $from.parent.content.size,
    '\n',
    leafText
  )
  const trigger = detectTrigger(before + after, before.length)
  if (!trigger) return null
  return {
    ...trigger,
    from: $from.pos - (before.length - trigger.start),
    to: $from.pos + (trigger.end - before.length)
  }
}

const SVG = 'http://www.w3.org/2000/svg'

const Mention = Node.create({
  name: 'mention',
  group: 'inline',
  inline: true,
  atom: true,
  selectable: false,
  addAttributes() {
    return {
      kind: { default: 'file' },
      value: { default: '' },
      attachment: { default: null, rendered: false },
      label: { default: null, rendered: false },
      detail: { default: null, rendered: false },
      src: { default: null, rendered: false }
    }
  },
  parseHTML() {
    return [
      {
        tag: 'span[data-mention]',
        getAttrs: (element) => ({
          kind: element.dataset.mention,
          value: element.dataset.value
        })
      }
    ]
  },
  renderHTML({ node }) {
    const kind = node.attrs.kind as MentionKind
    const value = String(node.attrs.value)
    if (kind === 'image') {
      const name = String(node.attrs.label ?? 'image')
      return [
        'span',
        {
          'data-mention': kind,
          'data-value': value,
          class: cn('rounded-sm px-0.5', CHIP_CLASSES.image)
        },
        [
          'img',
          {
            src: node.attrs.src ?? '',
            alt: '',
            class:
              'me-1 inline-block size-3.5 rounded-[3px] object-cover align-[-2px]'
          }
        ],
        name,
        [
          'span',
          { class: 'text-muted-foreground ms-1 text-[0.85em]' },
          String(node.attrs.detail ?? '')
        ]
      ]
    }
    const label =
      kind === 'file'
        ? `@${value.split('/').pop() || value}`
        : mentionSource(kind, value)
    const attributes = {
      'data-mention': kind,
      'data-value': value,
      title: kind === 'file' ? value : undefined,
      class: cn('rounded-sm px-0.5', CHIP_CLASSES[kind])
    }
    if (kind !== 'file') return ['span', attributes, label]
    const icon = fileIcon(value)
    return [
      'span',
      attributes,
      [
        `${SVG} svg`,
        {
          'aria-hidden': 'true',
          viewBox: '0 0 16 16',
          class: cn('me-0.5 inline-block size-3.5 align-[-2px]', icon.color)
        },
        [`${SVG} use`, { href: `#${icon.name}` }]
      ],
      label
    ]
  },
  renderText({ node }) {
    if (node.attrs.kind === 'image') return `[${node.attrs.label ?? 'image'}]`
    return mentionSource(node.attrs.kind, String(node.attrs.value))
  }
})

/** Shows the `/command` a prompt starts with as a chip. */
const LeadingCommand = Extension.create({
  name: 'leadingCommand',
  addProseMirrorPlugins() {
    return [
      new Plugin({
        key: new PluginKey('leadingCommand'),
        props: {
          decorations(state) {
            const first = state.doc.firstChild
            if (!first?.isTextblock || first.type.name === 'codeBlock') {
              return null
            }
            const command = /^\/[\w:-]+(?=\s|$)/.exec(first.textContent)
            if (!command) return null
            return DecorationSet.create(state.doc, [
              Decoration.inline(1, 1 + command[0].length, {
                class: cn('rounded-sm', CHIP_CLASSES.command)
              })
            ])
          }
        }
      })
    ]
  }
})

/** Enter sends; Shift+Enter does what Enter does in a document. */
const ComposerKeys = Extension.create({
  name: 'composerKeys',
  priority: 1000,
  addKeyboardShortcuts() {
    return {
      'Shift-Enter': ({ editor }) =>
        editor.commands.first(({ commands }) => [
          () => commands.newlineInCode(),
          () => commands.splitListItem('listItem'),
          () => commands.createParagraphNear(),
          () => commands.liftEmptyBlock(),
          () => commands.splitBlock()
        ])
    }
  }
})

const CONTENT_CLASSES = cn(
  'px-2.5 font-(family-name:--font-composer) wrap-break-word whitespace-pre-wrap text-(length:--composer-font-size) outline-none',
  '[&_p]:my-0 [&_strong]:font-semibold',
  '[&_blockquote]:border-foreground/25 [&_blockquote]:text-muted-foreground [&_blockquote]:my-1 [&_blockquote]:border-s-2 [&_blockquote]:ps-3',
  '[&_code]:bg-foreground/10 [&_code]:rounded [&_code]:px-[0.35em] [&_code]:py-[0.1em] [&_code]:font-mono [&_code]:text-[0.85em]',
  '[&_pre_code]:bg-transparent [&_pre_code]:p-0 [&_pre_code]:text-xs [&_pre_span]:text-(--shiki-light) dark:[&_pre_span]:text-(--shiki-dark)',
  '[&_ol]:my-1 [&_ol]:list-[tight-decimal] [&_ol]:ps-5 [&_ol>li]:ps-1.5 [&_ul]:my-1 [&_ul]:list-disc [&_ul]:ps-5',
  '[&_h1]:text-xl [&_h1]:font-semibold [&_h2]:text-lg [&_h2]:font-semibold [&_h3]:text-base [&_h3]:font-semibold',
  '[&_hr]:border-border [&_hr]:my-2',
  '[&_p.is-editor-empty:first-child]:before:text-muted-foreground [&_p.is-editor-empty:first-child]:before:pointer-events-none [&_p.is-editor-empty:first-child]:before:float-left [&_p.is-editor-empty:first-child]:before:h-0 [&_p.is-editor-empty:first-child]:before:content-[attr(data-placeholder)]'
)

/** The composer's rich text: markdown as you type it, and mentions as chips. */
export function ComposerEditor({
  ref,
  placeholder,
  label,
  className,
  contentClassName,
  onChange,
  initialContent,
  onDocument,
  onBlur,
  onKeyDown,
  onImages
}: ComposerEditorProps) {
  const handlers = useRef({ onChange, onDocument, onBlur, onKeyDown, onImages })
  handlers.current = { onChange, onDocument, onBlur, onKeyDown, onImages }
  /** The last document read, so a caret move doesn't convert it again. */
  const lastRead = useRef<{
    doc: ProseMirrorNode
    json: JSONContent
    text: string
  } | null>(null)

  const read = (editor: { state: EditorState; getJSON: () => JSONContent }) => {
    const { doc } = editor.state
    if (lastRead.current?.doc !== doc) {
      const json = editor.getJSON()
      lastRead.current = { doc, json, text: toMarkdown(json) }
    }
    return lastRead.current
  }

  const emit = (editor: {
    state: EditorState
    getJSON: () => JSONContent
    isEmpty: boolean
    isFocused: boolean
  }) =>
    handlers.current.onChange({
      text: read(editor).text,
      empty: editor.isEmpty,
      trigger: locateTrigger(editor.state),
      focused: editor.isFocused
    })

  const keyContext = (view: EditorView): KeyContext => {
    const { $from } = view.state.selection
    return {
      inCodeBlock: $from.parent.type.name === 'codeBlock',
      onFirstLine: () => $from.index(0) === 0 && view.endOfTextblock('up'),
      onLastLine: () =>
        $from.index(0) === view.state.doc.childCount - 1 &&
        view.endOfTextblock('down')
    }
  }

  const editor = useEditor({
    autofocus: 'end',
    content: initialContent,
    extensions: [
      StarterKit.configure({ link: false, underline: false, codeBlock: false }),
      ComposerCodeBlock,
      CodeBlockHighlight,
      Placeholder.configure({
        placeholder: ({ node }) =>
          node.type.name === 'paragraph' ? placeholder : ''
      }),
      Mention,
      LeadingCommand,
      ComposerKeys
    ],
    editorProps: {
      attributes: {
        role: 'combobox',
        'aria-label': label,
        'aria-multiline': 'true',
        'aria-autocomplete': 'list',
        'aria-expanded': 'false',
        class: cn(CONTENT_CLASSES, contentClassName)
      },
      handleKeyDown: (view, event) =>
        openFencedBlock(view, event) ||
        handlers.current.onKeyDown(event, keyContext(view)),
      // Never left to WebKit, which pastes an image as an attachment element of its own.
      handlePaste: (_view, event) => {
        const data = event.clipboardData
        const pasted = data ? pastedImages(data, { orClipboard: true }) : null
        if (!pasted) return false
        handlers.current.onImages(pasted)
        return true
      },
      // The form around the editor takes dropped images; the editor only stays out of it.
      handleDrop: (_view, event) =>
        event.dataTransfer !== null &&
        pastedImages(event.dataTransfer, { orClipboard: false }) !== null
    },
    onCreate: ({ editor }) => {
      if (!editor.isEmpty) emit(editor)
    },
    onUpdate: ({ editor }) => {
      emit(editor)
      handlers.current.onDocument?.(editor.isEmpty ? null : read(editor).json)
    },
    onSelectionUpdate: ({ editor }) => emit(editor),
    onFocus: ({ editor }) => emit(editor),
    onBlur: ({ editor }) => {
      emit(editor)
      handlers.current.onBlur?.()
    }
  })

  useImperativeHandle(
    ref,
    () => ({
      focus: () => {
        editor?.commands.focus()
      },
      paste: (text) => {
        editor?.view.pasteText(text)
      },
      insertImages: (images) => {
        if (!editor || editor.isEmpty || images.length === 0) return
        editor
          .chain()
          .focus()
          .insertContent(
            images.flatMap((image) => [
              {
                type: 'mention',
                attrs: {
                  kind: 'image',
                  value: image.key,
                  label: image.name,
                  detail: image.detail,
                  src: image.previewUrl
                }
              },
              { type: 'text', text: ' ' }
            ])
          )
          .run()
      },
      removeImage: (key) => {
        if (!editor) return
        const chips: number[] = []
        editor.state.doc.descendants((node, position) => {
          const chip =
            node.type.name === 'mention' &&
            node.attrs.kind === 'image' &&
            node.attrs.value === key
          if (chip) chips.push(position)
        })
        if (chips.length === 0) return
        const transaction = editor.state.tr
        for (const position of chips.reverse()) {
          transaction.delete(position, position + 1)
        }
        editor.view.dispatch(transaction)
      },
      setText: (text) => {
        if (!editor) return
        editor.commands.setContent(fromText(text))
        editor.commands.focus('end')
        emit(editor)
      },
      document: () => (!editor || editor.isEmpty ? null : editor.getJSON()),
      setDocument: (doc) => {
        if (!editor) return
        editor.commands.setContent(doc)
        editor.commands.focus('end')
        emit(editor)
      },
      replace: (trigger, insert) => {
        if (!editor) return
        const $to = editor.state.doc.resolve(trigger.to)
        const spaceAfter =
          $to.parent.textBetween(
            $to.parentOffset,
            Math.min($to.parentOffset + 1, $to.parent.content.size)
          ) === ' '
        const to = spaceAfter ? trigger.to + 1 : trigger.to
        const range = { from: trigger.from, to }
        const chain = editor.chain().focus()
        if ('remove' in insert) {
          chain.deleteRange(range).run()
          return
        }
        if ('mention' in insert) {
          chain
            .insertContentAt(range, [
              { type: 'mention', attrs: insert.mention },
              { type: 'text', text: ' ' }
            ])
            .run()
          return
        }
        const text = insert.keepOpen ? insert.text : `${insert.text} `
        chain
          .insertContentAt(
            insert.keepOpen ? { from: trigger.from, to: trigger.to } : range,
            { type: 'text', text }
          )
          .run()
      },
      mentions: () => {
        const found: MessageAttachment[] = []
        editor?.state.doc.descendants((node) => {
          const attachment = node.attrs.attachment as MessageAttachment | null
          if (node.type.name === 'mention' && attachment) found.push(attachment)
        })
        return found.filter(
          (attachment, index) =>
            found.findIndex(
              (other) => JSON.stringify(other) === JSON.stringify(attachment)
            ) === index
        )
      },
      setMenu: ({ listId, activeId }) => {
        const dom = editor?.view.dom
        if (!dom) return
        dom.setAttribute('aria-expanded', String(listId !== null))
        if (listId) dom.setAttribute('aria-controls', listId)
        else dom.removeAttribute('aria-controls')
        if (activeId) dom.setAttribute('aria-activedescendant', activeId)
        else dom.removeAttribute('aria-activedescendant')
      }
    }),
    [editor]
  )

  return <EditorContent editor={editor} className={className} />
}
