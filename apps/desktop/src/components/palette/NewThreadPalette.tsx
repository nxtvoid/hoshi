import { type ReactNode, useState } from 'react'
import { m } from '@/paraglide/messages'
import { useAppStore } from '@/state/app-store'
import { useStartThreadIn } from './use-new-thread'
import { HugeiconsIcon } from '@hugeicons/react'
import { ProjectIcon } from '@/components/sidebar/ProjectIcon'
import { BubbleChatIcon, Folder01Icon } from '@hugeicons/core-free-icons'
import {
  PALETTE_ICON,
  type PaletteEntry,
  type PaletteGroup,
  PaletteFrame
} from './PaletteFrame'

/**
 * Picks where a new thread starts: no project at all for a chat, set apart at the top, or a
 * project, the selected one first.
 */
export function NewThreadPalette({
  onClose,
  onBack
}: {
  onClose: () => void
  onBack: () => void
}) {
  const projects = useAppStore((state) => state.projects)
  const selectedProjectId = useAppStore((state) => state.selectedProjectId)
  const startThreadIn = useStartThreadIn()
  const [query, setQuery] = useState('')

  const needle = query.trim().toLowerCase()
  const matches = (...fields: string[]) =>
    fields.some((field) => field.toLowerCase().includes(needle))

  const projectEntry = (project: (typeof projects)[number]): PaletteEntry => ({
    key: project.id,
    run: () => startThreadIn(project.id),
    content: (
      <Row
        icon={
          <ProjectIcon
            projectId={project.id}
            className='size-4 rounded-[25%]'
            fallback={
              <HugeiconsIcon
                icon={Folder01Icon}
                strokeWidth={2}
                className={PALETTE_ICON}
              />
            }
          />
        }
        title={project.title}
        description={project.workspaceRoot}
      />
    )
  })
  const chatEntry: PaletteEntry = {
    key: 'chat',
    run: () => startThreadIn(null),
    content: (
      <Row
        icon={
          <HugeiconsIcon
            icon={BubbleChatIcon}
            strokeWidth={2}
            className={PALETTE_ICON}
          />
        }
        title={m.no_project()}
        description={m.no_project_description()}
      />
    )
  }

  const projectEntries = [
    ...projects.filter((project) => project.id === selectedProjectId),
    ...projects.filter((project) => project.id !== selectedProjectId)
  ]
    .filter((project) => matches(project.title, project.workspaceRoot))
    .map(projectEntry)
  const showChat = matches(m.no_project(), m.chat(), m.no_project_description())
  const groups: PaletteGroup[] = [
    ...(showChat
      ? [
          {
            label: '',
            entries: [chatEntry],
            divider: projectEntries.length > 0
          }
        ]
      : []),
    ...(projectEntries.length > 0
      ? [{ label: m.palette_projects(), entries: projectEntries }]
      : [])
  ]

  return (
    <PaletteFrame
      query={query}
      onQueryChange={setQuery}
      placeholder={m.new_thread_in_placeholder()}
      groups={groups}
      empty={m.palette_no_results()}
      onClose={onClose}
      onBack={onBack}
      enterLabel={m.palette_select()}
    />
  )
}

function Row({
  icon,
  title,
  description
}: {
  icon: ReactNode
  title: string
  description: string
}) {
  return (
    <>
      {icon}
      <span className='flex min-w-0 flex-1 flex-col'>
        <span className='truncate font-medium'>{title}</span>
        <span className='text-muted-foreground truncate text-xs'>
          {description}
        </span>
      </span>
    </>
  )
}
