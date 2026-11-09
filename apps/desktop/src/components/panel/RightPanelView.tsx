import { useState } from 'react'
import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { SurfaceLauncher } from './SurfaceLauncher'
import type { Surface } from '@/state/panel-surfaces'
import { Button } from '@hoshi/ui/components/button'
import { IconButton } from '@hoshi/ui/components/icon-button'
import { ResizeHandle } from '@hoshi/ui/components/resize-handle'
import {
  isAvailable,
  kindForShortcut,
  useSurfaceContext,
  renderSurface,
  SURFACE_KINDS,
  SURFACES
} from './surfaces'
import {
  PANEL_WIDTH,
  usePanelStore,
  useThreadSurfaces
} from '@/state/panel-store'
import {
  Add01Icon,
  ArrowExpand01Icon,
  ArrowShrink01Icon,
  Cancel01Icon
} from '@hugeicons/core-free-icons'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuShortcut,
  DropdownMenuTrigger
} from '@hoshi/ui/components/dropdown-menu'

/**
 * The thread's side panel: tabs of open surfaces (changes, pull request, and later
 * terminal, files, browser, side chat), each kept mounted while the thread is open.
 */
export function RightPanelView({ threadId }: { threadId: string }) {
  const width = usePanelStore((state) => state.width)
  const setWidth = usePanelStore((state) => state.setWidth)
  const maximized = usePanelStore((state) => state.maximized)
  const toggleMaximized = usePanelStore((state) => state.toggleMaximized)
  const close = usePanelStore((state) => state.close)
  const { surfaces, activeId } = useThreadSurfaces(threadId)

  return (
    <aside
      style={maximized ? undefined : { width }}
      className={cn(
        'bg-background relative flex min-h-0 flex-col',
        maximized ? 'min-w-0 flex-1' : 'border-border shrink-0 border-l'
      )}
    >
      {maximized ? null : (
        <ResizeHandle
          edge='left'
          label={m.resize_panel()}
          value={width}
          min={PANEL_WIDTH.min}
          max={PANEL_WIDTH.max}
          defaultValue={PANEL_WIDTH.default}
          onChange={setWidth}
        />
      )}
      <div className='border-border flex h-9 shrink-0 items-center gap-1 border-b pr-1 pl-2'>
        <div
          role='tablist'
          className='flex min-w-0 flex-1 items-center gap-1 overflow-x-auto'
        >
          {surfaces.map((surface) => (
            <SurfaceTab
              key={surface.id}
              threadId={threadId}
              surface={surface}
              active={surface.id === activeId}
            />
          ))}
        </div>
        <AddSurfaceMenu threadId={threadId} />
        <IconButton
          label={maximized ? m.restore_panel() : m.maximize_panel()}
          onClick={toggleMaximized}
        >
          <HugeiconsIcon
            icon={maximized ? ArrowShrink01Icon : ArrowExpand01Icon}
            strokeWidth={2}
          />
        </IconButton>
        <IconButton label={m.close_panel()} onClick={close}>
          <HugeiconsIcon icon={Cancel01Icon} strokeWidth={2} />
        </IconButton>
      </div>
      {surfaces.length === 0 ? (
        <SurfaceLauncher threadId={threadId} />
      ) : (
        surfaces.map((surface) => (
          <div
            key={surface.id}
            role='tabpanel'
            hidden={surface.id !== activeId}
            className='flex min-h-0 flex-1 flex-col'
          >
            {renderSurface(threadId, surface)}
          </div>
        ))
      )}
    </aside>
  )
}

function SurfaceTab({
  threadId,
  surface,
  active
}: {
  threadId: string
  surface: Surface
  active: boolean
}) {
  const activateSurface = usePanelStore((state) => state.activateSurface)
  const closeSurface = usePanelStore((state) => state.closeSurface)
  const { label, icon, Title } = SURFACES[surface.kind]

  return (
    <div
      className={cn(
        'group flex h-6 shrink-0 items-center rounded-md text-xs',
        active
          ? 'bg-secondary text-secondary-foreground'
          : 'text-muted-foreground hover:bg-accent/60'
      )}
    >
      <button
        type='button'
        role='tab'
        aria-selected={active}
        onClick={() => activateSurface(threadId, surface.id)}
        onAuxClick={(event) => {
          if (event.button === 1) closeSurface(threadId, surface.id)
        }}
        className='focus-visible:ring-ring flex h-full items-center gap-1.5 rounded-md pl-2 outline-none focus-visible:ring-2'
      >
        <HugeiconsIcon icon={icon} strokeWidth={2} className='size-3.5' />
        {Title ? <Title threadId={threadId} /> : label()}
      </button>
      <button
        type='button'
        aria-label={m.close_surface({ surface: label() })}
        onClick={() => closeSurface(threadId, surface.id)}
        className='hover:bg-accent focus-visible:ring-ring mx-0.5 flex size-4 items-center justify-center rounded-sm opacity-60 outline-none group-hover:opacity-100 focus-visible:opacity-100 focus-visible:ring-2'
      >
        <HugeiconsIcon icon={Cancel01Icon} strokeWidth={2} className='size-3' />
      </button>
    </div>
  )
}

function AddSurfaceMenu({ threadId }: { threadId: string }) {
  const [open, setOpen] = useState(false)
  const openSurface = usePanelStore((state) => state.openSurface)
  const context = useSurfaceContext(threadId)

  return (
    <DropdownMenu open={open} onOpenChange={setOpen}>
      <DropdownMenuTrigger
        render={
          <Button
            variant='ghost'
            size='icon-sm'
            aria-label={m.open_surface()}
          />
        }
      >
        <HugeiconsIcon icon={Add01Icon} strokeWidth={2} />
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align='end'
        className='w-48'
        onKeyDown={(event) => {
          const kind = kindForShortcut(event.key, context)
          if (!kind || event.ctrlKey || event.altKey || event.metaKey) return
          event.preventDefault()
          openSurface(threadId, kind)
          setOpen(false)
        }}
      >
        {SURFACE_KINDS.map((kind) => {
          const { label, icon, shortcut } = SURFACES[kind]
          return (
            <DropdownMenuItem
              key={kind}
              disabled={!isAvailable(kind, context)}
              onClick={() => openSurface(threadId, kind)}
            >
              <HugeiconsIcon icon={icon} strokeWidth={2} />
              {label()}
              <DropdownMenuShortcut>{shortcut}</DropdownMenuShortcut>
            </DropdownMenuItem>
          )
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
