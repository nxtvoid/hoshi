import { describe, expect, test } from 'bun:test'
import { editMenu, editShortcut, isTextInputType } from '@/lib/edit-menu'

describe('editMenu', () => {
  test('offers nothing on plain UI without a selection', () => {
    expect(editMenu('readOnly', false)).toBeNull()
  })

  test('only copies a selection outside an editor', () => {
    expect(editMenu('readOnly', true)).toEqual({
      cut: false,
      copy: true,
      paste: false,
      selectAll: false
    })
  })

  test('cuts and copies only with a selection in an editor', () => {
    expect(editMenu('text', false)).toEqual({
      cut: false,
      copy: false,
      paste: true,
      selectAll: true
    })
    expect(editMenu('text', true)).toEqual({
      cut: true,
      copy: true,
      paste: true,
      selectAll: true
    })
  })

  test('never cuts from the terminal', () => {
    expect(editMenu('terminal', true)).toEqual({
      cut: false,
      copy: true,
      paste: true,
      selectAll: true
    })
  })
})

describe('editShortcut', () => {
  test('follows the platform', () => {
    expect(editShortcut('paste', true)).toBe('⌘V')
    expect(editShortcut('selectAll', false)).toBe('Ctrl A')
  })
})

describe('isTextInputType', () => {
  test('text fields but not checkboxes or buttons', () => {
    expect(isTextInputType('text')).toBe(true)
    expect(isTextInputType('Search')).toBe(true)
    expect(isTextInputType('checkbox')).toBe(false)
    expect(isTextInputType('submit')).toBe(false)
  })
})
