import { describe, expect, test } from 'bun:test'
import {
  isChangesCardOpen,
  OPEN_CHANGES_LIMIT,
  useTurnChangesStore
} from '@/state/turn-changes-store'

describe('isChangesCardOpen', () => {
  test('opens small change sets by default', () => {
    expect(isChangesCardOpen(undefined, 1)).toBe(true)
    expect(isChangesCardOpen(undefined, OPEN_CHANGES_LIMIT)).toBe(true)
  })

  test('closes large change sets by default', () => {
    expect(isChangesCardOpen(undefined, OPEN_CHANGES_LIMIT + 1)).toBe(false)
    expect(isChangesCardOpen(undefined, 4000)).toBe(false)
  })

  test("the user's choice wins over the default", () => {
    expect(isChangesCardOpen(true, 4000)).toBe(true)
    expect(isChangesCardOpen(false, 3)).toBe(false)
  })
})

describe('useTurnChangesStore', () => {
  test('remembers each card on its own', () => {
    const { setOpen } = useTurnChangesStore.getState()
    setOpen('turn-a', false)
    setOpen('turn-b', true)

    expect(useTurnChangesStore.getState().open).toEqual({
      'turn-a': false,
      'turn-b': true
    })
  })
})
