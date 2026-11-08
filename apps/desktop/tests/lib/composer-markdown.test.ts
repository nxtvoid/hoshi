import { expect, test } from 'bun:test'
import type { JSONContent } from '@tiptap/core'
import { fromText, toMarkdown } from '../../src/lib/composer-markdown'

const text = (value: string, ...marks: string[]): JSONContent => ({
  type: 'text',
  text: value,
  marks: marks.map((type) => ({ type }))
})
const paragraph = (...content: JSONContent[]): JSONContent => ({
  type: 'paragraph',
  content
})
const doc = (...content: JSONContent[]): JSONContent => ({
  type: 'doc',
  content
})

test('text is sent as typed, without escaping', () => {
  expect(toMarkdown(doc(paragraph(text('fix foo_bar, 2*3 < 7 & `x`'))))).toBe(
    'fix foo_bar, 2*3 < 7 & `x`'
  )
})

test('marks wrap their runs once, even across nested marks', () => {
  const content = doc(
    paragraph(
      text('use '),
      text('bold ', 'bold'),
      text('both', 'bold', 'italic'),
      text(' and '),
      text('code', 'code')
    )
  )
  expect(toMarkdown(content)).toBe('use **bold *both*** and `code`')
})

test('paragraphs are lines; other blocks are set apart', () => {
  const content = doc(
    paragraph(text('one')),
    paragraph(),
    paragraph(text('two')),
    {
      type: 'blockquote',
      content: [paragraph(text('quoted')), paragraph(text('more'))]
    },
    {
      type: 'codeBlock',
      attrs: { language: 'ts' },
      content: [text('const a = 1\nconst b = 2')]
    },
    paragraph(text('after'))
  )
  expect(toMarkdown(content)).toBe(
    'one\n\ntwo\n\n> quoted\n> more\n\n```ts\nconst a = 1\nconst b = 2\n```\n\nafter'
  )
})

test('lists, headings and rules', () => {
  const content = doc(
    { type: 'heading', attrs: { level: 2 }, content: [text('Plan')] },
    {
      type: 'orderedList',
      attrs: { start: 3 },
      content: [
        {
          type: 'listItem',
          content: [paragraph(text('first')), paragraph(text('wrapped'))]
        },
        {
          type: 'listItem',
          content: [
            paragraph(text('second')),
            {
              type: 'bulletList',
              content: [
                { type: 'listItem', content: [paragraph(text('nested'))] }
              ]
            }
          ]
        }
      ]
    },
    { type: 'horizontalRule' }
  )
  expect(toMarkdown(content)).toBe(
    '## Plan\n\n3. first\n   wrapped\n4. second\n\n   - nested\n\n---'
  )
})

test('chips are written the way the agent reads them', () => {
  const mention = (kind: string, value: string): JSONContent => ({
    type: 'mention',
    attrs: { kind, value }
  })
  const content = doc(
    paragraph(
      text('see '),
      mention('file', 'src/main.rs'),
      text(' '),
      mention('file', 'docs/My Notes.md'),
      text(' for '),
      mention('pullRequest', '12'),
      text(' with '),
      mention('skill', 'pdf'),
      { type: 'hardBreak' },
      text('done')
    )
  )
  expect(toMarkdown(content)).toBe(
    'see @src/main.rs @"docs/My Notes.md" for #12 with $pdf\ndone'
  )
})

test('a recalled prompt comes back as the same text', () => {
  const prompt = 'first line\n\n> not parsed, kept as typed'
  expect(toMarkdown(fromText(prompt))).toBe(prompt)
})
