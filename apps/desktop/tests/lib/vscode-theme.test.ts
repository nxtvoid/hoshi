import { describe, expect, test } from 'bun:test'
import { colorToHex } from '../../src/lib/color'
import type { ThemeExtension } from '../../src/bindings/ThemeExtension'
import {
  extensionThemes,
  humanizeThemeName,
  isVsCodeThemeFile,
  pairVsCodeThemes,
  parseJsonc,
  parseThemeImport,
  parseVsCodeThemeFile,
  resolveThemeLabelCollisions
} from '../../src/lib/vscode-theme'
import { ThemeError, themeModes } from '../../src/state/themes'

const night = {
  name: 'Night Owl',
  type: 'dark',
  colors: {
    'editor.background': '#011627',
    'editor.foreground': '#d6deeb',
    focusBorder: '#7e57c2',
    'sideBar.background': '#011627',
    'editorError.foreground': '#ef5350',
    'terminal.selectionBackground': '#1b90dd4d'
  },
  tokenColors: []
}

const extension: ThemeExtension = {
  id: 'acme.owls',
  collectionId: 'open-vsx:acme.owls',
  name: 'Owls',
  publisher: 'acme',
  description: '',
  downloadCount: 1,
  iconUrl: null,
  sourceUrl: null,
  manifestUrl: 'https://open-vsx.org/m',
  sha256Url: 'https://open-vsx.org/s',
  vsixUrl: 'https://open-vsx.org/v',
  version: '1.0.0',
  license: 'MIT'
}

describe('parseJsonc', () => {
  test('ignores comments and trailing commas, but not inside strings', () => {
    expect(
      parseJsonc('{\n // hi\n "a": "http://x", /* b */ "c": [1,],\n}')
    ).toEqual({ a: 'http://x', c: [1] })
  })
})

describe('isVsCodeThemeFile', () => {
  test('tells VS Code themes from hoshi theme files', () => {
    expect(isVsCodeThemeFile(night)).toBe(true)
    expect(
      isVsCodeThemeFile({ version: 1, colors: { 'editor.background': '#000' } })
    ).toBe(false)
  })
})

describe('parseVsCodeThemeFile', () => {
  test('maps the workbench colors onto the roles', () => {
    const theme = parseVsCodeThemeFile(night, 'VS Code theme')
    expect(theme.label).toBe('Night Owl')
    expect(theme.id).toBe('night-owl')
    expect(theme.appearance).toBe('dark')
    expect(themeModes(theme)).toEqual(['dark'])
    expect(colorToHex(theme.colors.background)).toBe('#011627')
    expect(colorToHex(theme.colors.primary)).toBe('#7e57c2')
    expect(colorToHex(theme.colors.foreground)).toBe('#d6deeb')
    expect(theme.colors.terminalSelection).toEndWith('/ 0.302)')
  })

  test('needs an editor background', () => {
    expect(() =>
      parseVsCodeThemeFile({ colors: { foreground: '#fff' } }, 'x')
    ).toThrow(ThemeError)
  })

  test('guesses the appearance from the background', () => {
    const theme = parseVsCodeThemeFile(
      { colors: { 'editor.background': '#fdf6e3' } },
      'Solarized'
    )
    expect(theme.appearance).toBe('light')
    expect(theme.label).toBe('Solarized')
  })
})

describe('parseThemeImport', () => {
  test('reads either format from text', () => {
    expect(parseThemeImport(JSON.stringify(night), 'x').id).toBe('night-owl')
    expect(
      parseThemeImport(
        '{ "version": 1, "name": "Mine", "appearance": "light", "colors": { "background": "#fff" } }',
        'x'
      ).id
    ).toBe('mine')
  })

  test('reports text that is not JSON', () => {
    expect(() => parseThemeImport('{ nope', 'x')).toThrow(ThemeError)
  })
})

describe('pairVsCodeThemes', () => {
  test('joins a light and a dark theme of the same name', () => {
    const dark = parseVsCodeThemeFile({ ...night, name: 'Owl Dark' }, 'x')
    const light = parseVsCodeThemeFile(
      {
        name: 'Owl Light',
        type: 'light',
        colors: { 'editor.background': '#fbfbfb' }
      },
      'x'
    )
    const [paired, ...rest] = pairVsCodeThemes([dark, light])
    expect(rest).toEqual([])
    expect(paired!.label).toBe('Owl')
    expect(themeModes(paired!)).toEqual(['light', 'dark'])
  })
})

describe('resolveThemeLabelCollisions', () => {
  test('renames after the file, then numbers', () => {
    const theme = parseVsCodeThemeFile(night, 'x')
    const resolved = resolveThemeLabelCollisions([
      { theme, sourceName: 'night-owl.json' },
      { theme, sourceName: 'night-owl-noitalic.json' },
      { theme }
    ])
    expect(resolved.map((entry) => entry.label)).toEqual([
      'Night Owl',
      'Night Owl Noitalic',
      'Night Owl 2'
    ])
  })
})

describe('extensionThemes', () => {
  test('gives each theme a stable id inside the collection', () => {
    const files = [
      {
        label: 'Owl Night',
        uiTheme: 'dark',
        sourcePath: 'extension/themes/night.json',
        colors: night.colors
      }
    ]
    const [theme] = extensionThemes(extension, files)
    expect(theme!.id).toMatch(/^ovx-theme-[0-9a-f]{12}$/)
    expect(extensionThemes(extension, files)[0]!.id).toBe(theme!.id)
    expect(theme!.collection).toEqual({
      id: 'open-vsx:acme.owls',
      label: 'Owls'
    })
  })
})

describe('humanizeThemeName', () => {
  test('turns file names into labels', () => {
    expect(humanizeThemeName('tokyo-night_storm')).toBe('Tokyo Night Storm')
    expect(humanizeThemeName('Already Nice')).toBe('Already Nice')
  })
})
