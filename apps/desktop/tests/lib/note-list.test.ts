import { describe, expect, test } from 'bun:test'
import { type ListedNotes, withSavedNote } from '../../src/lib/note-list'
import type { NoteSummary } from '../../src/bindings/NoteSummary'

const note = (
  id: string,
  projectId: string | null,
  minute: number,
  title = id
): NoteSummary => ({
  id,
  projectId,
  title,
  preview: '',
  updatedAt: `2026-11-09T10:${String(minute).padStart(2, '0')}:00.000Z`,
  credentials: 0
})

const listed: ListedNotes = {
  projectId: 'p',
  own: [note('b', 'p', 20), note('a', 'p', 10)],
  general: [note('g', null, 15)]
}

const ids = (notes: NoteSummary[]) => notes.map((each) => each.id)

describe('withSavedNote', () => {
  test('an edited note moves to where its last edit puts it, with its new title', () => {
    const next = withSavedNote(listed, note('a', 'p', 30, 'renamed'))

    expect(ids(next.own)).toEqual(['a', 'b'])
    expect(next.own[0]?.title).toBe('renamed')
    expect(next.general).toEqual(listed.general)
  })

  test('a new note joins the notes it belongs to', () => {
    expect(ids(withSavedNote(listed, note('n', null, 40)).general)).toEqual([
      'n',
      'g'
    ])
    expect(ids(withSavedNote(listed, note('n', 'p', 5)).own)).toEqual([
      'b',
      'a',
      'n'
    ])
  })

  test('a note moved elsewhere leaves the list, and one made general changes group', () => {
    expect(ids(withSavedNote(listed, note('a', 'q', 30)).own)).toEqual(['b'])
    const general = withSavedNote(listed, note('a', null, 30))

    expect(ids(general.own)).toEqual(['b'])
    expect(ids(general.general)).toEqual(['a', 'g'])
  })
})
