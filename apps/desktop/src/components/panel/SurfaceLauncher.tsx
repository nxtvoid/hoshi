import { useEffect } from 'react'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { Kbd } from '@hoshi/ui/components/kbd'
import { usePanelStore } from '@/state/panel-store'
import { useSidebarStore } from '@/state/sidebar-store'
import {
  isAvailable,
  kindForShortcut,
  SURFACE_KINDS,
  SURFACES,
  useSurfaceContext
} from './surfaces'

/** Shortcuts must not fire while the user types in the composer or a field. */
function isTyping(target: EventTarget | null) {
  return (
    target instanceof HTMLElement &&
    (target.isContentEditable ||
      ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName))
  )
}

/** Shown in an empty panel: every surface, each with a one-key shortcut. */
export function SurfaceLauncher({ threadId }: { threadId: string }) {
  const openSurface = usePanelStore((state) => state.openSurface)
  const context = useSurfaceContext(threadId)
  const notesOpen = useSidebarStore((state) => state.notesOpen)

  useEffect(() => {
    if (notesOpen) return
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.ctrlKey || event.altKey || event.metaKey) return
      if (isTyping(event.target)) return
      const kind = kindForShortcut(event.key, context)
      if (!kind) return
      event.preventDefault()
      openSurface(threadId, kind)
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [threadId, openSurface, context, notesOpen])

  return (
    <div className='flex flex-1 flex-col items-center justify-center gap-3 p-4'>
      <h2 className='text-sm font-semibold'>{m.open_surface()}</h2>
      <ul className='flex w-full max-w-60 flex-col'>
        {SURFACE_KINDS.map((kind) => {
          const { label, icon, shortcut } = SURFACES[kind]
          return (
            <li key={kind}>
              <button
                type='button'
                aria-keyshortcuts={shortcut}
                disabled={!isAvailable(kind, context)}
                onClick={() => openSurface(threadId, kind)}
                className='hover:bg-accent focus-visible:ring-ring flex w-full items-center gap-2.5 rounded-md px-2 py-1.5 text-left text-sm outline-none focus-visible:ring-2 disabled:pointer-events-none disabled:opacity-45'
              >
                <HugeiconsIcon icon={icon} strokeWidth={2} className='size-4' />
                <span className='flex-1'>{label()}</span>
                <Kbd>{shortcut}</Kbd>
              </button>
            </li>
          )
        })}
      </ul>
    </div>
  )
}
