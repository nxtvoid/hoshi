import { describe, expect, test } from 'bun:test'
import {
  addTerminal,
  closeTerminal,
  NO_DRAWER,
  toggleDrawer
} from '@/state/terminal-drawer'

function ids() {
  let next = 0
  return () => `term-${++next}`
}

describe('toggleDrawer', () => {
  test('opening an empty drawer starts a terminal', () => {
    const drawer = toggleDrawer(NO_DRAWER, ids())

    expect(drawer).toEqual({
      open: true,
      terminalIds: ['term-1'],
      activeId: 'term-1'
    })
  })

  test('hiding and showing again keeps the terminals', () => {
    const makeId = ids()
    const shown = toggleDrawer(NO_DRAWER, makeId)

    const again = toggleDrawer(toggleDrawer(shown, makeId), makeId)

    expect(again).toEqual(shown)
  })
})

describe('closeTerminal', () => {
  test('closing the active terminal focuses its right neighbour', () => {
    const makeId = ids()
    let drawer = addTerminal(addTerminal(NO_DRAWER, makeId), makeId)
    drawer = addTerminal(drawer, makeId)
    drawer = { ...drawer, activeId: 'term-2' }

    expect(closeTerminal(drawer, 'term-2').activeId).toBe('term-3')
  })

  test('closing the last terminal hides the drawer', () => {
    const drawer = toggleDrawer(NO_DRAWER, ids())

    expect(closeTerminal(drawer, 'term-1')).toEqual(NO_DRAWER)
  })
})
