import { describe, expect, test } from 'bun:test'
import {
  CODE_SIZE,
  clampSize,
  DEFAULT_APPEARANCE,
  INTERFACE_SIZE,
  normalizeAppearance,
  resolveDark,
  rootAppearance
} from '../../src/state/appearance'

describe('normalizeAppearance', () => {
  test('keeps the theme an earlier version saved and fills the rest', () => {
    expect(normalizeAppearance({ preference: 'dark' })).toEqual({
      ...DEFAULT_APPEARANCE,
      preference: 'dark'
    })
  })

  test('falls back to the defaults without saved state', () => {
    expect(normalizeAppearance(undefined)).toEqual(DEFAULT_APPEARANCE)
    expect(normalizeAppearance(null)).toEqual(DEFAULT_APPEARANCE)
    expect(normalizeAppearance('dark')).toEqual(DEFAULT_APPEARANCE)
  })

  test('drops values it does not know', () => {
    expect(
      normalizeAppearance({
        preference: 'sepia',
        palette: 'neon',
        interfaceFont: 'comic',
        codeFont: 7,
        lighterText: 'no'
      })
    ).toEqual(DEFAULT_APPEARANCE)
  })

  test('keeps valid choices', () => {
    const saved = {
      preference: 'light',
      palette: 'iris',
      interfaceFont: 'inter',
      interfaceSize: 14,
      codeFont: 'cascadia',
      codeSize: 15,
      lighterText: false
    }
    expect(normalizeAppearance(saved)).toEqual(saved)
  })

  test('clamps sizes into their range', () => {
    const appearance = normalizeAppearance({ interfaceSize: 40, codeSize: 2 })
    expect(appearance.interfaceSize).toBe(INTERFACE_SIZE.max)
    expect(appearance.codeSize).toBe(CODE_SIZE.min)
  })
})

describe('clampSize', () => {
  test('rounds to whole pixels', () => {
    expect(clampSize(14.6, INTERFACE_SIZE)).toBe(15)
  })

  test('uses the default for anything but a finite number', () => {
    expect(clampSize(Number.NaN, CODE_SIZE)).toBe(CODE_SIZE.default)
    expect(clampSize('14', CODE_SIZE)).toBe(CODE_SIZE.default)
  })
})

describe('resolveDark', () => {
  test('an explicit theme ignores the system', () => {
    expect(resolveDark('dark', false)).toBe(true)
    expect(resolveDark('light', true)).toBe(false)
  })

  test('system follows the system', () => {
    expect(resolveDark('system', true)).toBe(true)
    expect(resolveDark('system', false)).toBe(false)
  })
})

describe('rootAppearance', () => {
  test('the defaults leave the stylesheet alone', () => {
    expect(rootAppearance(DEFAULT_APPEARANCE)).toEqual({
      palette: undefined,
      font: undefined,
      codeFont: undefined,
      textWeight: undefined,
      fontSize: undefined,
      codeFontSize: undefined
    })
  })

  test('maps each choice to the root', () => {
    expect(
      rootAppearance({
        preference: 'dark',
        palette: 'grove',
        interfaceFont: 'system',
        interfaceSize: 14,
        codeFont: 'cascadia',
        codeSize: 15,
        lighterText: false
      })
    ).toEqual({
      palette: 'grove',
      font: 'system',
      codeFont: 'cascadia',
      textWeight: 'regular',
      fontSize: '14px',
      codeFontSize: '15px'
    })
  })
})
