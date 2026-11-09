import { isMac } from '@/components/brand/EditorIcon'
import { usePaletteStore } from '@/state/palette-store'
import { lazy, Suspense, useEffect, useState } from 'react'

const PaletteDialog = lazy(() =>
  import('./PaletteDialog').then((module) => ({
    default: module.PaletteDialog
  }))
)

export const PALETTE_SHORTCUT = () => (isMac() ? '⌘K' : 'Ctrl K')

/** The palette's shortcut; the palette itself loads the first time it opens. */
export function CommandPalette() {
  const open = usePaletteStore((state) => state.open)
  const toggle = usePaletteStore((state) => state.toggle)
  const [opened, setOpened] = useState(open)

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const modifier = isMac() ? event.metaKey : event.ctrlKey
      if (!modifier || event.altKey || event.shiftKey) return
      if (event.code !== 'KeyK') return
      event.preventDefault()
      toggle()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [toggle])

  if (open && !opened) setOpened(true)
  if (!opened) return null
  return (
    <Suspense fallback={null}>
      <PaletteDialog />
    </Suspense>
  )
}
