import { expect, test } from 'bun:test'
import type { JSONContent } from '@tiptap/core'
import {
  fromText,
  imageChipKeys,
  joinDocuments,
  numberImageChips,
  toMarkdown,
  withoutImageChips
} from '../../src/lib/composer-markdown'

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

const A = '0b1e6a3c-1111-4aaa-8bbb-000000000001'
const B = '0b1e6a3c-2222-4aaa-8bbb-000000000002'

test('image chips are numbered as their images are sent', () => {
  const doc: JSONContent = {
    type: 'doc',
    content: [
      {
        type: 'paragraph',
        content: [
          { type: 'text', text: 'compare ' },
          { type: 'mention', attrs: { kind: 'image', value: B } },
          { type: 'text', text: ' with ' },
          { type: 'mention', attrs: { kind: 'image', value: A } },
          { type: 'text', text: ' and ' },
          { type: 'mention', attrs: { kind: 'image', value: B } },
          { type: 'text', text: ' ' },
          { type: 'mention', attrs: { kind: 'image', value: 'c0ffee' } }
        ]
      }
    ]
  }
  const text = toMarkdown(doc)

  expect(imageChipKeys(text)).toEqual([B, A, 'c0ffee'])
  expect(
    numberImageChips(
      text,
      new Map([
        [B, 1],
        [A, 2]
      ])
    )
  ).toBe('compare [Image #1] with [Image #2] and [Image #1]')
})

test('a saved draft drops its image chips', () => {
  const draft: JSONContent = {
    type: 'doc',
    content: [
      {
        type: 'paragraph',
        content: [
          { type: 'text', text: 'see ' },
          { type: 'mention', attrs: { kind: 'image', value: A } },
          { type: 'mention', attrs: { kind: 'file', value: 'src/a.ts' } }
        ]
      }
    ]
  }

  expect(toMarkdown(withoutImageChips(draft))).toBe('see @src/a.ts')
})

test('a failed message goes back above what was typed since', () => {
  const failed = doc(
    paragraph(text('look at '), {
      type: 'mention',
      attrs: { kind: 'image', value: A }
    })
  )
  const typed = doc(paragraph(text('and this')))

  expect(joinDocuments(failed, typed)).toEqual(
    doc(...(failed.content ?? []), ...(typed.content ?? []))
  )
  expect(joinDocuments(failed, null)).toBe(failed)
  expect(joinDocuments(null, typed)).toBe(typed)
  expect(joinDocuments(null, null)).toBeNull()
})
