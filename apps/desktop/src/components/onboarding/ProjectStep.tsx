import { useState } from 'react'
import { m } from '@/paraglide/messages'
import { useAppStore } from '@/state/app-store'
import { HugeiconsIcon } from '@hugeicons/react'
import { open } from '@tauri-apps/plugin-dialog'
import { loadedSettings } from '@/state/settings-store'
import { useSidebarStore } from '@/state/sidebar-store'
import type { CommandError } from '@/bindings/CommandError'
import { WizardDescription, WizardTitle } from '@hoshi/ui/components/wizard'
import {
  FolderAddIcon,
  FolderOpenIcon,
  Tick02Icon
} from '@hugeicons/core-free-icons'

type ProjectStepProps = {
  projectId: string | null
  onAdded: (projectId: string | null) => void
  /** Leaves the setup for the palette that clones or creates a folder. */
  onAddOrClone: () => void
}

export function ProjectStep({
  projectId,
  onAdded,
  onAddOrClone
}: ProjectStepProps) {
  const addProjectAt = useAppStore((state) => state.addProjectAt)
  const title = useAppStore(
    (state) =>
      state.projects.find((project) => project.id === projectId)?.title ?? null
  )
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const openFolder = async () => {
    if (pending) return
    setPending(true)
    setError(null)
    try {
      const folder = (await loadedSettings())?.general.projectsFolder?.trim()
      const picked = await open({
        directory: true,
        multiple: false,
        defaultPath: folder || undefined
      })
      if (!picked) return
      const failure = await addProjectAt(picked)
      if (failure) {
        setError(errorMessage(failure))
        return
      }
      useSidebarStore.getState().setView('project')
      onAdded(useAppStore.getState().selectedProjectId)
    } catch {
      setError(m.error_generic())
    } finally {
      setPending(false)
    }
  }

  return (
    <>
      <WizardTitle>{m.onboarding_project_title()}</WizardTitle>
      <WizardDescription>
        {m.onboarding_project_description()}
      </WizardDescription>
      <div className='mt-6 grid grid-cols-2 gap-3'>
        <ProjectOption
          icon={FolderOpenIcon}
          title={m.onboarding_open_folder()}
          description={m.onboarding_open_folder_description()}
          disabled={pending}
          onClick={() => void openFolder()}
        />
        <ProjectOption
          icon={FolderAddIcon}
          title={m.onboarding_add_or_clone()}
          description={m.onboarding_add_or_clone_description()}
          disabled={pending}
          onClick={onAddOrClone}
        />
      </div>
      {error ? (
        <p role='alert' className='text-destructive mt-4 text-sm'>
          {error}
        </p>
      ) : title ? (
        <p role='status' className='mt-4 flex items-center gap-1.5 text-sm'>
          <HugeiconsIcon
            icon={Tick02Icon}
            strokeWidth={2}
            className='text-status-done size-4'
          />
          {m.onboarding_project_added({ title })}
        </p>
      ) : null}
    </>
  )
}

function ProjectOption({
  icon,
  title,
  description,
  disabled,
  onClick
}: {
  icon: typeof FolderOpenIcon
  title: string
  description: string
  disabled: boolean
  onClick: () => void
}) {
  return (
    <button
      type='button'
      disabled={disabled}
      onClick={onClick}
      className='bg-background hover:bg-muted/50 focus-visible:ring-ring flex cursor-pointer items-start gap-3 rounded-xl border p-4 text-left transition-colors outline-none focus-visible:ring-2 disabled:pointer-events-none disabled:opacity-50'
    >
      <span className='bg-muted text-foreground grid size-9 shrink-0 place-items-center rounded-lg'>
        <HugeiconsIcon icon={icon} strokeWidth={2} className='size-4.5' />
      </span>
      <span className='min-w-0'>
        <span className='block text-sm font-medium'>{title}</span>
        <span className='text-muted-foreground mt-0.5 block text-xs'>
          {description}
        </span>
      </span>
    </button>
  )
}

function errorMessage(error: CommandError) {
  switch (error.code) {
    case 'folderUnavailable':
    case 'workspaceRootNotFound':
      return m.error_folder_unavailable({ path: error.path })
    default:
      return m.error_generic()
  }
}
