import { SortMenu } from './SortMenu'
import { useNewThread } from '@/components/palette/use-new-thread'
import { m } from '@/paraglide/messages'
import { Kbd } from '@hoshi/ui/components/kbd'
import { usePaletteStore } from '@/state/palette-store'
import { useAppStore } from '@/state/app-store'
import { HugeiconsIcon } from '@hugeicons/react'
import { Wordmark } from '@/components/brand/Logo'
import { useNotesStore } from '@/state/notes-store'
import { useSidebarStore } from '@/state/sidebar-store'
import { IconButton } from '@hoshi/ui/components/icon-button'
import { PALETTE_SHORTCUT } from '@/components/palette/CommandPalette'
import {
  FolderAddIcon,
  PencilEdit02Icon,
  Search01Icon,
  SidebarLeftIcon
} from '@hugeicons/core-free-icons'

export function SidebarHeader() {
  const notesOpen = useSidebarStore((state) => state.notesOpen)
  const threadSearch = useAppStore((state) => state.search)
  const setThreadSearch = useAppStore((state) => state.setSearch)
  const noteSearch = useNotesStore((state) => state.query)
  const setNoteSearch = useNotesStore((state) => state.setQuery)
  const openNote = useNotesStore((state) => state.open)
  const search = notesOpen ? noteSearch : threadSearch
  const setSearch = notesOpen ? setNoteSearch : setThreadSearch
  const searchLabel = notesOpen ? m.notes_search() : m.search_threads()
  const toggleSidebar = useAppStore((state) => state.toggleSidebar)
  const newThread = useNewThread()
  const hasProjects = useAppStore((state) => state.projects.length > 0)
  const openPalette = usePaletteStore((state) => state.setOpen)

  return (
    <header className='border-sidebar-border bg-starfield flex flex-col border-b px-2 pb-2.5'>
      <div data-tauri-drag-region className='flex h-10 items-center gap-1'>
        <IconButton label={m.toggle_sidebar()} onClick={toggleSidebar}>
          <HugeiconsIcon icon={SidebarLeftIcon} strokeWidth={2} />
        </IconButton>
        <span
          data-tauri-drag-region
          role='img'
          aria-label={m.app_name()}
          className='flex items-center px-1 select-none'
        >
          <Wordmark className='pointer-events-none h-3 w-auto' />
        </span>
      </div>
      <div className='flex items-center gap-1'>
        <label className='text-sidebar-foreground/70 hover:bg-sidebar-accent/60 focus-within:bg-sidebar-accent flex h-8 min-w-0 flex-1 items-center gap-2 rounded-lg px-2 text-sm'>
          <HugeiconsIcon
            icon={Search01Icon}
            strokeWidth={2}
            className='size-4 shrink-0'
          />
          <input
            type='search'
            value={search}
            onChange={(event) => setSearch(event.currentTarget.value)}
            placeholder={searchLabel}
            aria-label={searchLabel}
            className='placeholder:text-sidebar-foreground/50 min-w-0 flex-1 bg-transparent outline-none'
          />
          {search === '' ? (
            <button
              type='button'
              title={m.palette_label()}
              aria-label={m.palette_label()}
              onClick={() => openPalette(true)}
              className='shrink-0 cursor-pointer'
            >
              <Kbd>{PALETTE_SHORTCUT()}</Kbd>
            </button>
          ) : null}
        </label>
        {notesOpen ? null : <SortMenu />}
        <IconButton
          label={m.add_project()}
          onClick={() => openPalette(true, 'addProject')}
        >
          <HugeiconsIcon icon={FolderAddIcon} strokeWidth={2} />
        </IconButton>
        {notesOpen ? (
          <IconButton
            label={m.notes_new()}
            onClick={() => openNote(crypto.randomUUID())}
          >
            <HugeiconsIcon icon={PencilEdit02Icon} strokeWidth={2} />
          </IconButton>
        ) : (
          <IconButton
            label={m.new_thread()}
            onClick={() => void newThread()}
            disabled={!hasProjects}
          >
            <HugeiconsIcon icon={PencilEdit02Icon} strokeWidth={2} />
          </IconButton>
        )}
      </div>
    </header>
  )
}
