import { expect, test } from 'bun:test'
import { fitInside } from '../../src/lib/browser-layout'

test('a page wider than the panel shrinks to fit it whole', () => {
  expect(
    fitInside({ width: 720, height: 800 }, { width: 1440, height: 900 })
  ).toEqual({ width: 720, height: 450 })
})

test('a page taller than the panel shrinks by its height', () => {
  expect(
    fitInside({ width: 800, height: 422 }, { width: 390, height: 844 })
  ).toEqual({ width: 195, height: 422 })
})

test('a page smaller than the panel is never enlarged', () => {
  expect(
    fitInside({ width: 900, height: 900 }, { width: 390, height: 844 })
  ).toEqual({ width: 390, height: 844 })
})
