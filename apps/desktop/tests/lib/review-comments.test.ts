import { describe, expect, test } from 'bun:test'
import type { ReviewThread } from '../../src/bindings/ReviewThread'
import { formatReviewThreads } from '../../src/lib/review-comments'

const comment = (author: string | null, body: string) => ({
  author,
  body,
  createdAt: '2026-11-07T12:00:00Z',
  url: 'https://github.com/o/r/pull/1#c'
})

describe('formatReviewThreads', () => {
  test('lists unresolved threads with their location and comments', () => {
    const threads: ReviewThread[] = [
      {
        path: 'src/a.ts',
        line: 12,
        resolved: false,
        comments: [
          comment('ana', 'Rename this.\nIt is unclear.'),
          comment(null, 'Agreed')
        ]
      },
      {
        path: 'src/b.ts',
        line: 3,
        resolved: true,
        comments: [comment('luis', 'Done')]
      },
      {
        path: 'README.md',
        line: null,
        resolved: false,
        comments: [comment('ana', 'Typo')]
      }
    ]

    expect(formatReviewThreads(threads)).toBe(
      [
        '- src/a.ts:12',
        '  - ana: Rename this.',
        '    It is unclear.',
        '  - ?: Agreed',
        '- README.md',
        '  - ana: Typo'
      ].join('\n')
    )
  })
})
