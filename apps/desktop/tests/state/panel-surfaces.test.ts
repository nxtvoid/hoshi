import { describe, expect, test } from 'bun:test'
import {
  activateSurface,
  closeSurface,
  NO_SURFACES,
  openSurface,
  showTurnDiff,
  type ThreadSurfaces,
  updateSurface
} from '../../src/state/panel-surfaces'

const ids = () => {
  let next = 0
  return () => `s${++next}`
}

const kinds = (panel: ThreadSurfaces) =>
  panel.surfaces.map((surface) => surface.kind)

describe('openSurface', () => {
  test('adds a surface with its defaults and focuses it', () => {
    const panel = openSurface(NO_SURFACES, 'diff', ids())

    expect(panel.surfaces).toEqual([
      { id: 's1', kind: 'diff', scope: 'uncommitted' }
    ])
    expect(panel.activeId).toBe('s1')
  })

  test('a single-instance kind already open is focused, not duplicated', () => {
    const makeId = ids()
    let panel = openSurface(NO_SURFACES, 'pullRequest', makeId)
    panel = openSurface(panel, 'diff', makeId)

    panel = openSurface(panel, 'pullRequest', makeId)

    expect(kinds(panel)).toEqual(['pullRequest', 'diff'])
    expect(panel.activeId).toBe('s1')
  })
})

describe('closeSurface', () => {
  const three = () => {
    const makeId = ids()
    let panel = openSurface(NO_SURFACES, 'diff', makeId)
    panel = openSurface(panel, 'pullRequest', makeId)
    return activateSurface(panel, 's1')
  }

  test('closing the active one focuses its right neighbour', () => {
    expect(closeSurface(three(), 's1').activeId).toBe('s2')
  })

  test('closing the last active one focuses the left neighbour', () => {
    const panel = activateSurface(three(), 's2')

    expect(closeSurface(panel, 's2').activeId).toBe('s1')
  })

  test('closing an inactive one keeps the focus', () => {
    expect(closeSurface(three(), 's2').activeId).toBe('s1')
  })

  test('closing everything leaves nothing active', () => {
    const panel = closeSurface(closeSurface(three(), 's1'), 's2')

    expect(panel).toEqual(NO_SURFACES)
  })
})

test('updateSurface changes only that surface', () => {
  const panel = openSurface(NO_SURFACES, 'diff', ids())

  const changed = updateSurface(panel, 's1', (surface) =>
    surface.kind === 'diff' ? { ...surface, scope: 'branch' } : surface
  )

  expect(changed.surfaces[0]).toEqual({
    id: 's1',
    kind: 'diff',
    scope: 'branch'
  })
})

describe('showTurnDiff', () => {
  test('opens the diff surface on a turn, focused on a file', () => {
    const panel = showTurnDiff(NO_SURFACES, 'm1', 'src/a.ts', ids())

    expect(panel.surfaces).toEqual([
      {
        id: 's1',
        kind: 'diff',
        scope: 'turn',
        turnId: 'm1',
        focusPath: 'src/a.ts'
      }
    ])
    expect(panel.activeId).toBe('s1')
  })

  test('reuses the diff surface already open, whatever it showed', () => {
    const makeId = ids()
    const opened = openSurface(
      openSurface(NO_SURFACES, 'diff', makeId),
      'terminal',
      makeId
    )

    const panel = showTurnDiff(opened, 'm2', undefined, makeId)

    expect(panel.surfaces[0]).toEqual({
      id: 's1',
      kind: 'diff',
      scope: 'turn',
      turnId: 'm2',
      focusPath: undefined
    })
    expect(panel.activeId).toBe('s1')
  })
})
