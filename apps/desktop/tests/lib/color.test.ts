import { describe, expect, test } from 'bun:test'
import {
  BLACK,
  canonicalColor,
  colorToHex,
  contrastRatio,
  isColor,
  parseColor,
  readableOn,
  WHITE
} from '../../src/lib/color'

describe('parseColor', () => {
  test('reads hex in every length', () => {
    expect(colorToHex('#fff')).toBe('#ffffff')
    expect(colorToHex('#ff000080')).toBe('#ff000080')
    expect(colorToHex('#1b4ed8')).toBe('#1b4ed8')
  })

  test('reads rgb, hsl and oklch', () => {
    expect(colorToHex('rgb(255 0 0)')).toBe('#ff0000')
    expect(colorToHex('rgba(0, 0, 255, 0.5)')).toBe('#0000ff80')
    expect(colorToHex('hsl(120 100% 50%)')).toBe('#00ff00')
    expect(colorToHex('oklch(1 0 0)')).toBe('#ffffff')
    expect(parseColor('oklch(1 0 0 / 10%)')?.alpha).toBeCloseTo(0.1)
  })

  test('rejects what is not a literal color', () => {
    expect(isColor('red')).toBe(false)
    expect(isColor('var(--primary)')).toBe(false)
    expect(isColor('#12')).toBe(false)
    expect(isColor('rgb(1 2)')).toBe(false)
    expect(isColor(12)).toBe(false)
  })
})

describe('canonicalColor', () => {
  test('stores colors as oklch and survives a round trip', () => {
    const canonical = canonicalColor('#1b4ed8')
    expect(canonical).toStartWith('oklch(')
    expect(colorToHex(canonical!)).toBe('#1b4ed8')
  })

  test('keeps the alpha', () => {
    expect(canonicalColor('#00000080')).toEndWith('/ 0.502)')
  })
})

describe('readableOn', () => {
  test('leaves a readable color alone', () => {
    expect(readableOn('#000000', '#ffffff', 4.5)).toBe('#000000')
  })

  test('darkens a color that is too light on a light surface', () => {
    const fixed = readableOn('#ffee88', '#ffffff', 4.5)
    const rgb = parseColor(fixed)!
    expect(rgb).not.toBeNull()
    const hex = colorToHex(fixed)!
    const channel = (index: number) =>
      Number.parseInt(hex.slice(1 + index * 2, 3 + index * 2), 16)
    expect(
      contrastRatio({ r: channel(0), g: channel(1), b: channel(2) }, WHITE)
    ).toBeGreaterThanOrEqual(4.5)
  })
})

describe('contrastRatio', () => {
  test('black on white is 21', () => {
    expect(contrastRatio(BLACK, WHITE)).toBeCloseTo(21)
  })
})
