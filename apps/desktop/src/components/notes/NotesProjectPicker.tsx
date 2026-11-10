import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { useAppStore } from '@/state/app-store'
import { useNotesStore } from '@/state/notes-store'
import { useNotesProjectId } from '@/state/notes-hub'
import { Note01Icon } from '@hugeicons/core-free-icons'
import { ProjectIcon } from '@/components/sidebar/ProjectIcon'
import {
  Combobox,
  ComboboxContent,
  ComboboxEmpty,
  ComboboxItem,
  ComboboxList,
  ComboboxSearch,
  ComboboxTrigger
} from '@hoshi/ui/components/combobox'

/** Stands for the general notes among the choices, which are project ids. */
const GENERAL = ''

/**
 * The project whose notes are listed, and the way to see another one's, or the general
 * ones alone. It moves no note: a note keeps the project it belongs to.
 */
export function NotesProjectPicker() {
  const projects = useAppStore((state) => state.projects)
  const list = useNotesStore((state) => state.list)
  const current = useNotesProjectId() ?? GENERAL
  const titles = new Map(projects.map((project) => [project.id, project.title]))
  const labelOf = (choice: string) => titles.get(choice) ?? m.notes_general()

  return (
    <Combobox
      items={[GENERAL, ...titles.keys()]}
      autoHighlight
      value={current}
      onValueChange={(next: string | null) => {
        if (next !== null && next !== current) {
          list(next === GENERAL ? null : next)
        }
      }}
      itemToStringLabel={labelOf}
    >
      <ComboboxTrigger
        size='sm'
        aria-label={m.notes_listed_for({ place: labelOf(current) })}
        className='text-muted-foreground hover:text-foreground max-w-52 shrink-0 gap-1 pr-1.5 pl-2 text-xs data-[size=sm]:h-6 data-[size=sm]:rounded-full'
      >
        <span className='flex min-w-0 items-center gap-1.5'>
          <NotePlaceIcon projectId={current || null} title={labelOf(current)} />
          <span className='truncate'>{labelOf(current)}</span>
        </span>
      </ComboboxTrigger>
      <ComboboxContent className='w-60'>
        <ComboboxSearch
          placeholder={m.notes_search_projects()}
          aria-label={m.notes_search_projects()}
        />
        <ComboboxEmpty>{m.notes_no_projects_match()}</ComboboxEmpty>
        <ComboboxList className='max-h-64 overflow-y-auto overscroll-contain p-1'>
          {(choice: string) => (
            <ComboboxItem key={choice} value={choice} className='gap-2 py-1.5'>
              <NotePlaceIcon
                projectId={choice || null}
                title={labelOf(choice)}
              />
              <span className='min-w-0 truncate'>{labelOf(choice)}</span>
            </ComboboxItem>
          )}
        </ComboboxList>
      </ComboboxContent>
    </Combobox>
  )
}

/** A project's own icon, or its initials; the notes' icon for the general ones. */
export function NotePlaceIcon({
  projectId,
  title
}: {
  projectId: string | null
  title: string
}) {
  if (projectId === null) {
    return (
      <HugeiconsIcon
        icon={Note01Icon}
        strokeWidth={2}
        className='text-muted-foreground size-4 shrink-0'
      />
    )
  }
  return (
    <ProjectIcon
      projectId={projectId}
      className='size-4 rounded-[25%]'
      fallback={
        <span className='bg-muted text-muted-foreground flex size-4 shrink-0 items-center justify-center rounded-[25%] text-[0.5rem] font-semibold uppercase'>
          {title.slice(0, 2)}
        </span>
      }
    />
  )
}
