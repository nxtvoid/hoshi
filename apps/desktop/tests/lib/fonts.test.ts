import { describe, expect, test } from 'bun:test'
import {
  firstAvailableFamily,
  fontPickerValues,
  installedFamilies
} from '../../src/lib/fonts'

const FONTS = [
  { family: 'Arial', monospace: false },
  { family: 'Cascadia Code', monospace: true },
  { family: 'Consolas', monospace: true },
  { family: 'Inter', monospace: false },
  { family: 'Segoe UI', monospace: false }
]

const PRESETS = [
  { value: 'geist', label: 'Geist' },
  { value: 'inter', label: 'Inter' }
]

describe('installedFamilies', () => {
  test('lists every family but the bundled ones', () => {
    expect(installedFamilies(FONTS, false)).toEqual([
      'Arial',
      'Cascadia Code',
      'Consolas',
      'Segoe UI'
    ])
  })

  test('keeps only fixed-width families for code', () => {
    expect(installedFamilies(FONTS, true)).toEqual([
      'Cascadia Code',
      'Consolas'
    ])
  })
})

describe('fontPickerValues', () => {
  test('puts the presets before the installed families', () => {
    expect(fontPickerValues(PRESETS, ['Arial', 'Segoe UI'], '')).toEqual([
      'geist',
      'inter',
      'Arial',
      'Segoe UI'
    ])
  })

  test('filters presets and families by name, ignoring case', () => {
    expect(
      fontPickerValues(PRESETS, ['Arial', 'Bridge Sans', 'Georgia'], ' GE ')
    ).toEqual(['geist', 'Bridge Sans', 'Georgia'])
  })

  test('returns nothing when no name matches', () => {
    expect(fontPickerValues(PRESETS, ['Arial'], 'zzz')).toEqual([])
  })

  test('a family named like a preset does not repeat it', () => {
    expect(fontPickerValues(PRESETS, ['geist', 'Arial'], '')).toEqual([
      'geist',
      'inter',
      'Arial'
    ])
  })
})

describe('firstAvailableFamily', () => {
  const stack =
    '"JetBrains Mono", "Cascadia Mono", ui-monospace, Consolas, monospace'

  test('returns the first installed family of the stack', () => {
    const installed = new Set(['Cascadia Mono', 'Consolas'])
    expect(firstAvailableFamily(stack, (name) => installed.has(name))).toBe(
      'Cascadia Mono'
    )
  })

  test('skips generic families', () => {
    expect(firstAvailableFamily(stack, () => false)).toBeNull()
    expect(
      firstAvailableFamily('system-ui, sans-serif', (name) => name !== '')
    ).toBeNull()
  })
})
