import { expect, test } from 'bun:test'
import { promptSegments } from '../../src/lib/prompt-segments'

const screenshot = {
  type: 'image' as const,
  id: 'image-1',
  name: 'screenshot.png',
  mimeType: 'image/png',
  size: 2048
}

const report = {
  type: 'file' as const,
  id: 'file-1',
  name: 'report.pdf',
  mimeType: 'application/pdf',
  size: 4096
}

const known = {
  pullRequests: new Set([12]),
  skills: new Set(['pdf', 'anthropic-skills:docx']),
  images: [screenshot],
  files: [report]
}

const chips = (text: string) =>
  promptSegments(text, known)
    .filter((segment) => segment.kind !== 'text')
    .map((segment) => [segment.kind, segment.text])

test('the segments cover the whole prompt in order', () => {
  const text = '/review #12 with @src/main.rs, then $pdf'
  const segments = promptSegments(text, known)

  expect(segments.map((segment) => segment.text).join('')).toBe(text)
  expect(chips(text)).toEqual([
    ['command', '/review'],
    ['pullRequest', '#12'],
    ['file', '@src/main.rs'],
    ['skill', '$pdf']
  ])
})

test('only known pull requests and skills become chips', () => {
  expect(
    chips('see #7 and #12. costs $5, use $other or $anthropic-skills:docx')
  ).toEqual([
    ['pullRequest', '#12'],
    ['skill', '$anthropic-skills:docx']
  ])
})

test('mentions inside words and commands mid-prompt stay text', () => {
  expect(chips('mail a@b.com about issue#12 and run /init')).toEqual([])
})

test('a quoted path with spaces is one chip', () => {
  expect(chips('open @"docs/My Notes.md" now')).toEqual([
    ['file', '@"docs/My Notes.md"']
  ])
})

test('an image the message sent shows as a chip with its name', () => {
  const segments = promptSegments('fix this[Image #1] and [Image #2]', known)
  expect(segments.filter((segment) => segment.kind === 'image')).toEqual([
    {
      kind: 'image',
      text: '[Image #1]',
      start: 8,
      end: 18,
      label: 'screenshot.png',
      image: screenshot
    }
  ])
})

test('a file the message sent shows as a chip with its name', () => {
  const segments = promptSegments('read [File #1], not [File #2]', known)
  expect(segments.filter((segment) => segment.kind === 'attachedFile')).toEqual(
    [
      {
        kind: 'attachedFile',
        text: '[File #1]',
        start: 5,
        end: 14,
        label: 'report.pdf',
        file: report
      }
    ]
  )
})
