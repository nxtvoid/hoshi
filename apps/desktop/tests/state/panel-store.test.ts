import { beforeEach, describe, expect, test } from 'bun:test'
import { usePanelStore } from '../../src/state/panel-store'

const opened = () => Object.keys(usePanelStore.getState().opened).sort()

describe('the panel of each thread', () => {
  beforeEach(() =>
    usePanelStore.setState({ opened: {}, threads: {}, maximized: false })
  )

  test('opens in the thread that asked and stays closed in the others', () => {
    usePanelStore.getState().openNote('a', 'm1', 'n1')

    expect(opened()).toEqual(['a'])
    expect(usePanelStore.getState().threads.b).toBeUndefined()
  })

  test('closing a thread’s last surface closes its panel only', () => {
    const { openSurface, closeSurface } = usePanelStore.getState()
    openSurface('a', 'diff')
    openSurface('b', 'diff')
    const surfaceId = usePanelStore.getState().threads.a?.activeId ?? ''

    closeSurface('a', surfaceId)

    expect(opened()).toEqual(['b'])
  })

  test('closed by hand, a thread keeps its surfaces for the next time', () => {
    const { openSurface, toggle } = usePanelStore.getState()
    openSurface('a', 'diff')

    toggle('a')
    expect(opened()).toEqual([])
    expect(usePanelStore.getState().threads.a?.surfaces).toHaveLength(1)

    toggle('a')
    expect(opened()).toEqual(['a'])
  })

  test('a deleted thread leaves nothing behind', () => {
    usePanelStore.getState().openSurface('a', 'diff')

    usePanelStore.getState().forget(['a'])

    expect(opened()).toEqual([])
    expect(usePanelStore.getState().threads).toEqual({})
  })
})
