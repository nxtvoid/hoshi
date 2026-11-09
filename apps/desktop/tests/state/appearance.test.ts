import { describe, expect, test } from 'bun:test'
import {
  CODE_SIZE,
  clampSize,
  DEFAULT_APPEARANCE,
  INTERFACE_SIZE,
  normalizeAppearance,
  rootAppearance
} from '../../src/state/appearance'
import { deriveThemeColors } from '../../src/state/themes'

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
        theme: 'neon',
        halves: { light: 'gone' },
        customThemes: [{ id: 'grove' }],
        interfaceFont: 'comic',
        codeFont: 7,
        lighterText: 'no'
      })
    ).toEqual(DEFAULT_APPEARANCE)
  })

  test('keeps valid choices', () => {
    const saved = {
      preference: 'light',
      theme: 'iris',
      halves: { dark: 'ocean' },
      customThemes: [],
      interfaceFont: 'inter',
      interfaceSize: 14,
      codeFont: 'cascadia',
      codeSize: 15,
      lighterText: false
    }
    expect(normalizeAppearance(saved)).toEqual(saved)
  })

  test('moves the palette an earlier version saved to the theme', () => {
    expect(normalizeAppearance({ palette: 'ember' }).theme).toBe('ember')
    expect(normalizeAppearance({ palette: 'neon' }).theme).toBe('hoshi')
  })

  test('keeps a custom theme it can read', () => {
    const custom = {
      id: 'dusk',
      label: 'Dusk',
      appearance: 'dark',
      colors: deriveThemeColors('dark', '#15131c', '#d08cff')
    }
    const appearance = normalizeAppearance({
      theme: 'dusk',
      halves: { light: 'grove', dark: 'dusk' },
      customThemes: [custom, { id: 'broken' }]
    })
    expect(appearance.theme).toBe('dusk')
    expect(appearance.halves).toEqual({ light: 'grove', dark: 'dusk' })
    expect(appearance.customThemes).toEqual([custom])
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

describe('rootAppearance', () => {
  test('the defaults leave the stylesheet alone', () => {
    expect(rootAppearance(DEFAULT_APPEARANCE, false)).toEqual({
      dark: false,
      palette: undefined,
      colors: undefined,
      font: undefined,
      codeFont: undefined,
      textWeight: undefined,
      fontSize: undefined,
      codeFontSize: undefined
    })
  })

  test('system follows the system, an explicit mode ignores it', () => {
    expect(rootAppearance(DEFAULT_APPEARANCE, true).dark).toBe(true)
    expect(
      rootAppearance({ ...DEFAULT_APPEARANCE, preference: 'light' }, true).dark
    ).toBe(false)
  })

  test('maps each choice to the root', () => {
    expect(
      rootAppearance(
        {
          ...DEFAULT_APPEARANCE,
          preference: 'dark',
          theme: 'grove',
          interfaceFont: 'system',
          interfaceSize: 14,
          codeFont: 'cascadia',
          codeSize: 15,
          lighterText: false
        },
        false
      )
    ).toEqual({
      dark: true,
      palette: 'grove',
      colors: undefined,
      font: 'system',
      codeFont: 'cascadia',
      textWeight: 'regular',
      fontSize: '14px',
      codeFontSize: '15px'
    })
  })

  test('paints each half with its own theme', () => {
    const colors = deriveThemeColors('dark', '#15131c', '#d08cff')
    const appearance = {
      ...DEFAULT_APPEARANCE,
      theme: 'ocean',
      halves: { dark: 'dusk' },
      customThemes: [
        { id: 'dusk', label: 'Dusk', appearance: 'dark' as const, colors }
      ]
    }
    expect(rootAppearance(appearance, false).palette).toBe('ocean')
    const dark = rootAppearance(appearance, true)
    expect(dark.palette).toBeUndefined()
    expect(dark.colors).toBe(colors)
  })
})
