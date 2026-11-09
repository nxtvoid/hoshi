import { cssFontFamily } from '@/state/appearance'

const PROBE_TEXT = 'mmmmmmmmMMWli1O0@# fjord'
const GENERICS = ['monospace', 'serif', 'sans-serif']

let context: CanvasRenderingContext2D | null | undefined

function probeWidth(fonts: string): number | null {
  if (context === undefined) {
    context = document.createElement('canvas').getContext('2d')
  }
  if (!context) return null
  context.font = `16px ${fonts}`
  return context.measureText(PROBE_TEXT).width
}

/** Measures text, since `document.fonts.check()` also accepts families that aren't installed. */
export function isFontFamilyAvailable(family: string): boolean {
  const name = cssFontFamily(family)
  try {
    return GENERICS.some((generic) => {
      const baseline = probeWidth(generic)
      const candidate = probeWidth(`${name}, ${generic}`)
      return baseline !== null && candidate !== null && candidate !== baseline
    })
  } catch {
    return false
  }
}
