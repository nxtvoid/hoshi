import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'

type ProjectIconState = {
  /** Data URL per project id; `null` when it has none, absent until asked. */
  icons: Record<string, string | null>
  load: (projectId: string) => void
}

const pending = new Set<string>()

export const useProjectIconStore = create<ProjectIconState>()((set, get) => ({
  icons: {},
  load: (projectId) => {
    if (projectId in get().icons || pending.has(projectId)) return
    pending.add(projectId)
    invoke<string | null>('get_project_icon', { projectId })
      .catch((error) => {
        console.error('hoshi: failed to load a project icon', error)
        return null
      })
      .then((icon) =>
        set((state) => ({ icons: { ...state.icons, [projectId]: icon } }))
      )
      .finally(() => pending.delete(projectId))
  }
}))
