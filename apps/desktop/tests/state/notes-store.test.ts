import { beforeEach, describe, expect, test } from 'bun:test'
import { type AttachedNote, useNotesStore } from '../../src/state/notes-store'

const note = (id: string, credentials = 0): AttachedNote => ({
  id,
  title: `Note ${id}`,
  credentials
})

const attached = (draftKey: string) =>
  (useNotesStore.getState().attached[draftKey] ?? []).map((each) => each.id)

describe('attached notes', () => {
  beforeEach(() => useNotesStore.setState({ attached: {}, revision: 0 }))

  test('a draft carries each note once, with its latest details', () => {
    const { attach } = useNotesStore.getState()
    attach('t1', note('a'))
    attach('t1', note('b'))
    attach('t1', note('a', 2))

    expect(attached('t1')).toEqual(['b', 'a'])
    expect(useNotesStore.getState().attached.t1?.[1]?.credentials).toBe(2)
    expect(attached('t2')).toEqual([])
  })

  test('detaching the last note leaves the draft without any', () => {
    const { attach, detach } = useNotesStore.getState()
    attach('t1', note('a'))

    detach('t1', 'a')

    expect('t1' in useNotesStore.getState().attached).toBe(false)
  })

  test('a deleted note leaves every draft that carried it', () => {
    const { attach, forget } = useNotesStore.getState()
    attach('t1', note('a'))
    attach('t1', note('b'))
    attach('t2', note('a'))

    forget('a')

    expect(attached('t1')).toEqual(['b'])
    expect('t2' in useNotesStore.getState().attached).toBe(false)
  })

  test('sending a draft clears its notes and no other', () => {
    const { attach, clearAttached } = useNotesStore.getState()
    attach('t1', note('a'))
    attach('t2', note('b'))

    clearAttached('t1')

    expect(attached('t1')).toEqual([])
    expect(attached('t2')).toEqual(['b'])
  })
})

describe('the listed notes', () => {
  beforeEach(() =>
    useNotesStore.setState({ listedProjectId: null, openNoteId: null })
  )

  test('listing the same project again keeps its note open', () => {
    const { list, open } = useNotesStore.getState()
    list('p')
    open('n1')

    list('p')

    expect(useNotesStore.getState().openNoteId).toBe('n1')
  })

  test('listing another project closes the note, unless one is asked for', () => {
    const { list, open } = useNotesStore.getState()
    list('p')
    open('n1')

    list('q')
    expect(useNotesStore.getState().openNoteId).toBeNull()

    list('p', 'n2')
    expect(useNotesStore.getState()).toMatchObject({
      listedProjectId: 'p',
      openNoteId: 'n2'
    })
  })

  test('a deleted note closes', () => {
    const { list, forget } = useNotesStore.getState()
    list(null, 'n1')

    forget('n1')

    expect(useNotesStore.getState().openNoteId).toBeNull()
  })
})
