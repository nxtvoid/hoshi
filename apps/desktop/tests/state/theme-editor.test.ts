import { describe, expect, test } from 'bun:test'
import {
  appearanceLock,
  findMergeTarget,
  saveEditor,
  startEditor
} from '../../src/state/theme-editor'
import {
  BUILT_IN_THEMES,
  deriveThemeColors,
  standardColors,
  type ThemeDefinition,
  ThemeError,
  themeModes
} from '../../src/state/themes'

const dusk: ThemeDefinition = {
  id: 'dusk',
  label: 'Dusk',
  appearance: 'dark',
  colors: deriveThemeColors('dark', '#15131c', '#d08cff'),
  managed: true
}

const grove = BUILT_IN_THEMES.find((theme) => theme.id === 'grove')!

const colors = {
  light: deriveThemeColors('light', '#fbf8f1', '#b4532a'),
  dark: deriveThemeColors('dark', '#101018', '#7c6cff')
}

const save = (overrides: Partial<Parameters<typeof saveEditor>[0]>) =>
  saveEditor({
    name: 'Aurora',
    editing: null,
    appearance: 'light',
    colors,
    advanced: false,
    simpleDirty: { light: false, dark: false },
    customThemes: [dusk],
    ...overrides
  })

describe('startEditor', () => {
  test('starts from hoshi in the simple mode', () => {
    const start = startEditor(null, 'dark')
    expect(start.appearance).toBe('dark')
    expect(start.advanced).toBe(false)
    expect(start.colors.dark).toEqual(standardColors('dark'))
  })

  test('the standard theme as a seed is the same as none', () => {
    expect(startEditor(BUILT_IN_THEMES[0]!, 'light').advanced).toBe(false)
  })

  test('a hand-tuned seed opens in the advanced mode', () => {
    const start = startEditor(grove, 'light')
    expect(start.advanced).toBe(true)
    expect(start.colors.light.primary).toBe(grove.colors.primary)
  })

  test('a seed without the wanted appearance opens on its own', () => {
    const start = startEditor(dusk, 'light')
    expect(start.appearance).toBe('dark')
    expect(start.advanced).toBe(false)
  })
})

describe('findMergeTarget', () => {
  test('matches a custom theme by name or id, but not the one being edited', () => {
    expect(findMergeTarget(' dusk ', [dusk], undefined)).toBe(dusk)
    expect(findMergeTarget('Dusk', [dusk], 'dusk')).toBeNull()
    expect(findMergeTarget('', [dusk], undefined)).toBeNull()
  })
})

describe('appearanceLock', () => {
  test('an edited theme keeps to its own appearances', () => {
    expect(appearanceLock('light', dusk, null)?.reason).toBe('editing_lacks')
    expect(appearanceLock('dark', dusk, null)).toBeNull()
  })

  test('a new theme cannot take a palette its namesake has', () => {
    expect(appearanceLock('dark', null, dusk)?.reason).toBe('merge_has')
    expect(appearanceLock('light', null, dusk)).toBeNull()
  })
})

describe('saveEditor', () => {
  test('creates a single-appearance theme and makes it active', () => {
    const result = save({})
    expect(result.created).toBe(true)
    expect(result.saved.id).toBe('aurora')
    expect(themeModes(result.saved)).toEqual(['light'])
    expect(result.saved.managed).toBe(true)
    expect(result.selection).toEqual({ theme: 'aurora', halves: {} })
    expect(result.themes.map((theme) => theme.id)).toEqual(['dusk', 'aurora'])
  })

  test('rebuilds the simple colors it changed', () => {
    const edited = {
      ...colors,
      light: { ...colors.light, background: '#ffffff', primary: '#ff0000' }
    }
    const result = save({
      colors: edited,
      simpleDirty: { light: true, dark: false }
    })
    expect(result.saved.colors).toEqual(
      deriveThemeColors('light', '#ffffff', '#ff0000')
    )
  })

  test('a name it already has adds the missing palette', () => {
    const result = save({ name: 'Dusk' })
    expect(result.mergedAppearance).toBe('light')
    expect(themeModes(result.saved)).toEqual(['light', 'dark'])
    expect(result.themes).toHaveLength(1)
  })

  test('refuses a palette its namesake already has', () => {
    expect(() => save({ name: 'Dusk', appearance: 'dark' })).toThrow(ThemeError)
  })

  test('an edit keeps its id and its choice', () => {
    const result = save({
      name: 'Dusk at sea',
      editing: dusk,
      appearance: 'dark',
      advanced: true
    })
    expect(result.saved.id).toBe('dusk')
    expect(result.saved.label).toBe('Dusk at sea')
    expect(result.saved.managed).toBeUndefined()
    expect(result.selection).toBeNull()
  })

  test('renaming an edit onto a namesake merges into it', () => {
    const dawn: ThemeDefinition = {
      id: 'dawn',
      label: 'Dawn',
      appearance: 'light',
      colors: colors.light
    }
    const result = save({
      name: 'Dusk',
      editing: dawn,
      customThemes: [dusk, dawn]
    })
    expect(result.mergedAppearance).toBe('light')
    expect(result.themes.map((theme) => theme.id)).toEqual(['dusk'])
    expect(themeModes(result.themes[0]!)).toEqual(['light', 'dark'])
  })

  test('needs a name', () => {
    expect(() => save({ name: '  ' })).toThrow(ThemeError)
  })
})
