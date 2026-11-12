import { describe, expect, test } from 'bun:test'
import {
  buildVisibleToastLayout,
  hasVisibleToastAction,
  shouldHideCollapsedToastContent,
  shouldRenderScopedToast
} from '../../src/lib/toast-layout'

describe('buildVisibleToastLayout', () => {
  test('stacks live toasts by their heights', () => {
    const layout = buildVisibleToastLayout([
      { id: 'a', height: 60 },
      { id: 'b', height: 40 }
    ])

    expect(layout.frontmostHeight).toBe(60)
    expect(
      layout.items.map(({ visibleIndex, offsetY }) => [visibleIndex, offsetY])
    ).toEqual([
      [0, 0],
      [1, 60]
    ])
  })

  test('a toast on its way out keeps its place; the live ones close up', () => {
    const layout = buildVisibleToastLayout([
      { id: 'leaving', height: 50, transitionStatus: 'ending' as const },
      { id: 'live', height: 30 }
    ])

    expect(layout.frontmostHeight).toBe(30)
    expect(
      layout.items.map(({ visibleIndex, offsetY }) => [visibleIndex, offsetY])
    ).toEqual([
      [0, 0],
      [0, 0]
    ])
  })

  test('an unmeasured toast counts as no height', () => {
    expect(
      buildVisibleToastLayout([{ height: Number.NaN }]).frontmostHeight
    ).toBe(0)
  })
})

describe('toast content', () => {
  test('only the frontmost of a collapsed stack shows its content', () => {
    expect(shouldHideCollapsedToastContent(0, 1)).toBe(false)
    expect(shouldHideCollapsedToastContent(0, 3)).toBe(false)
    expect(shouldHideCollapsedToastContent(2, 3)).toBe(true)
  })

  test('an action shows only with children', () => {
    expect(hasVisibleToastAction({ children: 'Retry' })).toBe(true)
    expect(hasVisibleToastAction({ children: '' })).toBe(false)
    expect(hasVisibleToastAction({})).toBe(false)
    expect(hasVisibleToastAction(undefined)).toBe(false)
  })

  test('a scoped toast shows only in its scope', () => {
    expect(shouldRenderScopedToast(undefined, null)).toBe(true)
    expect(shouldRenderScopedToast({ scope: 't1' }, 't1')).toBe(true)
    expect(shouldRenderScopedToast({ scope: 't1' }, 't2')).toBe(false)
    expect(shouldRenderScopedToast({ scope: 't1' }, null)).toBe(false)
  })
})
