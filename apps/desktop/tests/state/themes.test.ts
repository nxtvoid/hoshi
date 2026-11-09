import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { contrastRatio, colorToRgb, WHITE } from '../../src/lib/color'
import {
  BUILT_IN_PALETTE_IDS,
  chooseCard,
  chooseHalf,
  chooseTheme,
  deriveThemeColors,
  halfOwners,
  installTheme,
  PALETTE_TINTS,
  paletteColors,
  parseStoredTheme,
  parseThemeFile,
  replaceCollection,
  resolveThemeAppearance,
  resolveThemeFor,
  roleVariable,
  selectionWithout,
  serializeThemeFile,
  standardColors,
  STANDARD_THEME_ID,
  THEME_COLOR_ROLES,
  ThemeError,
  type ThemeDefinition,
  themeIdFromName,
  updateColorFamily,
  validHalves
} from '../../src/state/themes'

const darkOnly: ThemeDefinition = {
  id: 'midnight',
  label: 'Midnight',
  appearance: 'dark',
  colors: deriveThemeColors('dark', '#101018', '#7c6cff')
}

const lightOnly: ThemeDefinition = {
  id: 'paper',
  label: 'Paper',
  appearance: 'light',
  colors: deriveThemeColors('light', '#fbf8f1', '#b4532a')
}

const customs = [darkOnly, lightOnly]

function cssBlock(css: string, selector: string): Map<string, string> {
  const start = css.indexOf(`${selector} {`)
  const body = css.slice(start, css.indexOf('}', start))
  const values = new Map<string, string>()
  for (const match of body.matchAll(/(--[\w-]+):\s*([^;]+);/g)) {
    values.set(match[1]!, match[2]!.trim())
  }
  return values
}

describe('built-in colors', () => {
  const css = readFileSync(
    new URL('../../../../packages/ui/src/styles/globals.css', import.meta.url),
    'utf8'
  )

  test('the standard theme mirrors the stylesheet', () => {
    const light = cssBlock(css, ':root,\n.light')
    const dark = cssBlock(css, '.dark')
    for (const role of THEME_COLOR_ROLES) {
      expect(standardColors('light')[role]).toBe(light.get(roleVariable(role))!)
      expect(standardColors('dark')[role]).toBe(dark.get(roleVariable(role))!)
    }
  })

  test('the palettes mirror the stylesheet', () => {
    const light = cssBlock(css, '\n[data-palette]')
    const dark = cssBlock(css, '.dark[data-palette]')
    for (const id of BUILT_IN_PALETTE_IDS) {
      const { hue, chroma } = PALETTE_TINTS[id]
      const resolve = (value: string) =>
        value
          .replace('var(--palette-hue)', String(hue))
          .replace('var(--palette-chroma)', String(chroma))
      for (const [variable, value] of light) {
        const role = THEME_COLOR_ROLES.find((r) => roleVariable(r) === variable)
        if (role) expect(paletteColors(id, 'light')[role]).toBe(resolve(value))
      }
      for (const [variable, value] of dark) {
        const role = THEME_COLOR_ROLES.find((r) => roleVariable(r) === variable)
        if (role) expect(paletteColors(id, 'dark')[role]).toBe(resolve(value))
      }
    }
  })
})

describe('roleVariable', () => {
  test('names the CSS variable', () => {
    expect(roleVariable('sidebarPrimaryForeground')).toBe(
      '--sidebar-primary-foreground'
    )
    expect(roleVariable('background')).toBe('--background')
  })
})

describe('halves', () => {
  test('a whole theme clears the mix', () => {
    expect(chooseTheme('grove')).toEqual({ theme: 'grove', halves: {} })
  })

  test('an orb takes one half and leaves the other to the base', () => {
    const selection = chooseHalf(chooseTheme('grove'), 'dark', 'iris')
    expect(selection).toEqual({ theme: 'grove', halves: { dark: 'iris' } })
    expect(halfOwners(selection, [])).toEqual({ light: 'grove', dark: 'iris' })
  })

  test('the standard orb moves a themed base to the other half', () => {
    const selection = chooseHalf(chooseTheme('ocean'), 'light', null)
    expect(selection).toEqual({
      theme: STANDARD_THEME_ID,
      halves: { dark: 'ocean' }
    })
    expect(halfOwners(selection, [])).toEqual({ light: null, dark: 'ocean' })
  })

  test('the standard orb on a standard base clears that half', () => {
    const mixed = { theme: STANDARD_THEME_ID, halves: { light: 'ember' } }
    expect(chooseHalf(mixed, 'light', null)).toEqual({
      theme: STANDARD_THEME_ID,
      halves: {}
    })
  })

  test('a single-appearance card takes its half, a full one the whole', () => {
    const base = chooseTheme('grove')
    expect(chooseCard(base, darkOnly)).toEqual({
      theme: 'grove',
      halves: { dark: 'midnight' }
    })
    expect(
      chooseCard(base, {
        ...lightOnly,
        variants: { dark: darkOnly.colors }
      })
    ).toEqual({ theme: 'paper', halves: {} })
  })

  test('halves naming a missing theme or appearance are dropped', () => {
    expect(validHalves({ light: 'midnight', dark: 'gone' }, customs)).toEqual(
      {}
    )
    expect(validHalves({ light: 'paper', dark: 'midnight' }, customs)).toEqual({
      light: 'paper',
      dark: 'midnight'
    })
  })

  test('removing themes hands their halves back', () => {
    expect(
      selectionWithout({ theme: 'paper', halves: { dark: 'midnight' } }, [
        'paper'
      ])
    ).toEqual({ theme: STANDARD_THEME_ID, halves: { dark: 'midnight' } })
    expect(
      selectionWithout({ theme: 'grove', halves: { dark: 'midnight' } }, [
        'midnight'
      ])
    ).toEqual({ theme: 'grove', halves: {} })
  })
})

describe('resolveThemeAppearance', () => {
  test('follows the mode', () => {
    const selection = chooseTheme('grove')
    expect(resolveThemeAppearance(selection, 'system', true, [])).toBe('dark')
    expect(resolveThemeAppearance(selection, 'light', true, [])).toBe('light')
  })

  test('a single-appearance base keeps its own appearance', () => {
    const selection = chooseTheme('midnight')
    expect(resolveThemeAppearance(selection, 'light', false, customs)).toBe(
      'dark'
    )
  })

  test('a half for the wanted appearance wins over the base', () => {
    const selection = { theme: 'midnight', halves: { light: 'paper' } }
    expect(resolveThemeAppearance(selection, 'light', false, customs)).toBe(
      'light'
    )
    expect(resolveThemeFor(selection, 'light', customs).theme.id).toBe('paper')
    expect(resolveThemeFor(selection, 'dark', customs).theme.id).toBe(
      'midnight'
    )
  })
})

describe('theme files', () => {
  test('round-trip through serializeThemeFile', () => {
    const parsed = parseThemeFile(JSON.parse(serializeThemeFile(lightOnly)))
    expect(parsed.id).toBe('paper')
    expect(parsed.colors).toEqual(lightOnly.colors)
  })

  test('fill missing roles from the standard theme', () => {
    const theme = parseThemeFile({
      version: 1,
      name: 'Tiny',
      appearance: 'dark',
      colors: { background: '#000000' }
    })
    expect(theme.id).toBe('tiny')
    expect(theme.colors.foreground).toBe(standardColors('dark').foreground)
  })

  test('reject what they cannot use', () => {
    const code = (value: unknown) => {
      try {
        parseThemeFile(value)
      } catch (error) {
        return (error as ThemeError).code
      }
    }
    expect(code([])).toBe('not_an_object')
    expect(code({ version: 2 })).toBe('unsupported_version')
    expect(
      code({ version: 1, name: 'Grove', appearance: 'light', colors: {} })
    ).toBe('reserved_id')
    expect(
      code({
        version: 1,
        name: 'X',
        appearance: 'light',
        colors: { glow: '#fff' }
      })
    ).toBe('unknown_role')
    expect(
      code({
        version: 1,
        name: 'X',
        appearance: 'light',
        colors: { background: 'blue' }
      })
    ).toBe('invalid_color')
  })

  test('stored themes skip unknown roles instead of failing', () => {
    const stored = parseStoredTheme({
      ...lightOnly,
      colors: { ...lightOnly.colors, retired: '#fff' }
    })
    expect(stored?.colors).toEqual(lightOnly.colors)
    expect(parseStoredTheme({ ...lightOnly, id: 'grove' })).toBeNull()
  })
})

describe('library', () => {
  test('refuses a second theme with the same id', () => {
    expect(() => installTheme(customs, darkOnly)).toThrow(ThemeError)
  })

  test('replaces a collection in place', () => {
    const collection = { id: 'open-vsx:a.b', label: 'Pack' }
    const old: ThemeDefinition = { ...darkOnly, id: 'old', collection }
    const next: ThemeDefinition = { ...darkOnly, id: 'new', collection }
    expect(
      replaceCollection([lightOnly, old, darkOnly], collection.id, [next]).map(
        (theme) => theme.id
      )
    ).toEqual(['paper', 'new', 'midnight'])
  })
})

describe('themeIdFromName', () => {
  test('slugs names and hashes the ones without ASCII', () => {
    expect(themeIdFromName('  Tokyo Night! ')).toBe('tokyo-night')
    expect(themeIdFromName('夜')).toMatch(/^theme-[a-z0-9]+$/)
  })
})

describe('deriveThemeColors', () => {
  test('keeps text readable on the background', () => {
    for (const [appearance, background] of [
      ['light', '#fdfcf8'],
      ['dark', '#14121a']
    ] as const) {
      const colors = deriveThemeColors(appearance, background, '#2d7ff9')
      expect(
        contrastRatio(
          colorToRgb(colors.foreground, WHITE),
          colorToRgb(colors.background, WHITE)
        )
      ).toBeGreaterThanOrEqual(7)
    }
  })
})

describe('updateColorFamily', () => {
  test('the accent carries its foreground and rings along', () => {
    const colors = updateColorFamily(
      standardColors('light'),
      'primary',
      '#ffd400'
    )
    expect(colors.sidebarPrimary).toBe(colors.primary)
    expect(
      contrastRatio(
        colorToRgb(colors.primaryForeground, WHITE),
        colorToRgb(colors.primary, WHITE)
      )
    ).toBeGreaterThanOrEqual(4.5)
  })
})
