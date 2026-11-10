import { describe, expect, test } from 'bun:test'
import { noteReference, noteReferences } from '../../src/lib/note-reference'

const FIRST = '0b6a1f0e-52c3-4f5e-9d1c-6a7f3e2b9c10'
const SECOND = 'c41d7a55-0f2e-4b8a-8a3d-1e9b5f7d2a06'

describe('noteReferences', () => {
  test('a copied reference is read back as its note', () => {
    expect(noteReferences(noteReference(FIRST))).toEqual([FIRST])
    expect(noteReferences(`  ${noteReference(FIRST)}\n`)).toEqual([FIRST])
  })

  test('several references are read once each, in order', () => {
    const pasted = [FIRST, SECOND, FIRST].map(noteReference).join('\n')

    expect(noteReferences(pasted)).toEqual([FIRST, SECOND])
  })

  test('text with anything else is pasted as text', () => {
    expect(noteReferences(`see ${noteReference(FIRST)}`)).toBeNull()
    expect(noteReferences('hoshi://note/not-an-id')).toBeNull()
    expect(noteReferences(FIRST)).toBeNull()
    expect(noteReferences('   ')).toBeNull()
  })
})
