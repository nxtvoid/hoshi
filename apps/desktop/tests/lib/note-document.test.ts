import { describe, expect, test } from 'bun:test'
import { toMarkdown } from '../../src/lib/composer-markdown'
import {
  changedStretch,
  noteDocument,
  replaceInDocument
} from '../../src/lib/note-document'

const kinds = (text: string) =>
  (noteDocument(text).content ?? []).map((node) => node.type)

describe('noteDocument', () => {
  test('plain text reads back exactly, blank lines and markdown-looking text included', () => {
    const text = 'hola\n\nAPI_KEY:2312_dS*Ads*a\n# not a heading\n- nor a list'

    expect(toMarkdown(noteDocument(text))).toBe(text)
    expect(kinds(text)).toEqual(Array(5).fill('paragraph'))
  })

  test('what sits between fences is a code block in its language', () => {
    const text = 'run it\n\n```tsx\nconst a = 1\n\nexport {}\n```\n\ndone'
    const document = noteDocument(text)

    expect(kinds(text)).toEqual(['paragraph', 'codeBlock', 'paragraph'])
    expect(document.content?.[1]).toEqual({
      type: 'codeBlock',
      attrs: { language: 'tsx' },
      content: [{ type: 'text', text: 'const a = 1\n\nexport {}' }]
    })
    expect(toMarkdown(document)).toBe(text)
  })

  test('blank lines beyond the one around a code block stay', () => {
    const text = 'a\n\n\n```\nx\n```\n\n\nb'

    expect(toMarkdown(noteDocument(text))).toBe(text)
  })

  test('a fence that never closes stays as the text it is', () => {
    const text = 'see\n```js\nconst a = 1'

    expect(kinds(text)).toEqual(['paragraph', 'paragraph', 'paragraph'])
    expect(toMarkdown(noteDocument(text))).toBe(text)
  })
})

describe('replaceInDocument', () => {
  const document = noteDocument('db\npassword: hunter2\n\n```\nok\n```')

  test('the value changes in the run of text that holds it', () => {
    const replaced = replaceInDocument(document, 'hunter2', '[vault: DB]')

    expect(replaced && toMarkdown(replaced)).toBe(
      'db\npassword: [vault: DB]\n\n```\nok\n```'
    )
  })

  test('a value in no single run, or in several, is left alone', () => {
    expect(replaceInDocument(document, 'db\npassword', 'x')).toBeNull()
    expect(replaceInDocument(noteDocument('aa\naa'), 'aa', 'b')).toBeNull()
    expect(replaceInDocument(document, '', 'x')).toBeNull()
  })
})

describe('changedStretch', () => {
  test('finds what one text had where the other differs', () => {
    expect(
      changedStretch('password: hunter2\nok', 'password: [vault: DB]\nok')
    ).toEqual({ from: 'hunter2', to: '[vault: DB]' })
    expect(changedStretch('same', 'same')).toEqual({ from: '', to: '' })
  })
})
