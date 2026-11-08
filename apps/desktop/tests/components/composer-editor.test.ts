import { describe, expect, test } from 'bun:test'
import { Window } from 'happy-dom'

const window = new Window()
Object.assign(globalThis, {
  window,
  document: window.document,
  navigator: window.navigator,
  Node: window.Node,
  HTMLElement: window.HTMLElement,
  HTMLTemplateElement: window.HTMLTemplateElement,
  customElements: window.customElements,
  Element: window.Element,
  MutationObserver: window.MutationObserver,
  DOMParser: window.DOMParser,
  getComputedStyle: window.getComputedStyle.bind(window),
  requestAnimationFrame: (callback: () => void) => setTimeout(callback, 0),
  cancelAnimationFrame: (id: number) => clearTimeout(id)
})

const { Editor } = await import('@tiptap/core')
const { default: StarterKit } = await import('@tiptap/starter-kit')
const { default: CodeBlock } = await import('@tiptap/extension-code-block')
const { fromText, toMarkdown } = await import('../../src/lib/composer-markdown')
const { openFencedBlock } = await import(
  '../../src/components/thread/code-fence'
)

function editorWith(text: string) {
  const element = window.document.createElement('div')
  window.document.body.appendChild(element as unknown as Node)
  const editor = new Editor({
    element: element as unknown as HTMLElement,
    extensions: [StarterKit.configure({ codeBlock: false }), CodeBlock],
    content: fromText(text)
  })
  editor.commands.focus('end')
  return editor
}

const enter = (shiftKey: boolean) =>
  new window.KeyboardEvent('keydown', {
    key: 'Enter',
    shiftKey
  }) as unknown as KeyboardEvent

describe('code fences in the composer', () => {
  for (const shift of [false, true]) {
    test(`${shift ? 'Shift+Enter' : 'Enter'} after a fence opens a block in its language`, () => {
      const editor = editorWith('```tsx')

      expect(openFencedBlock(editor.view, enter(shift))).toBe(true)
      expect(editor.getJSON().content?.[0]).toEqual({
        type: 'codeBlock',
        attrs: { language: 'tsx' }
      })
      editor.commands.insertContent('const a = 1')
      expect(toMarkdown(editor.getJSON())).toBe('```tsx\nconst a = 1\n```')
    })
  }

  test('a fence without a language opens a plain block', () => {
    const editor = editorWith('```')

    expect(openFencedBlock(editor.view, enter(false))).toBe(true)
    expect(editor.getJSON().content?.[0]?.type).toBe('codeBlock')
  })

  test('other lines are left to the composer', () => {
    const editor = editorWith('see ```tsx')

    expect(openFencedBlock(editor.view, enter(false))).toBe(false)
  })

  test('a space after the fence also opens the block', () => {
    const editor = editorWith('```tsx')
    const { from, to } = editor.state.selection

    editor.view.someProp('handleTextInput', (handle) =>
      handle(editor.view, from, to, ' ', () => editor.state.tr)
    )

    expect(editor.getJSON().content?.[0]?.type).toBe('codeBlock')
  })
})
