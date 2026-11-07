import { useMemo } from 'react'
import { m } from '@/paraglide/messages'
import { ThreadCard } from './ThreadCard'
import { useAppStore } from '@/state/app-store'
import { Button } from '@hoshi/ui/components/button'
import { useSidebarStore } from '@/state/sidebar-store'

export function ThreadList() {
  const ready = useAppStore((state) => state.ready)
  const threads = useAppStore((state) => state.threads)
  const search = useAppStore((state) => state.search)
  const hasProjects = useAppStore((state) => state.projects.length > 0)
  const projectId = useAppStore((state) => state.selectedProjectId)
  const selectedThreadId = useAppStore((state) => state.selectedThreadId)
  const selectThread = useAppStore((state) => state.selectThread)
  const addProject = useAppStore((state) => state.addProject)
  const threadSort = useSidebarStore((state) => state.threadSort)

  const visible = useMemo(() => {
    const query = search.trim().toLowerCase()
    const key = threadSort === 'created' ? 'createdAt' : 'updatedAt'
    return threads
      .filter(
        (thread) =>
          thread.projectId === projectId &&
          (query === '' || thread.title.toLowerCase().includes(query))
      )
      .sort((a, b) => b[key].localeCompare(a[key]))
  }, [threads, search, projectId, threadSort])

  if (!ready) return <div className='flex-1' />
  if (!hasProjects) {
    return (
      <div className='flex flex-1 flex-col items-center gap-3 p-4 text-center'>
        <p className='text-muted-foreground text-xs'>{m.no_projects()}</p>
        <Button variant='outline' size='sm' onClick={addProject}>
          {m.add_project()}
        </Button>
      </div>
    )
  }
  if (!projectId) {
    return <EmptyMessage>{m.no_project_selected()}</EmptyMessage>
  }
  if (visible.length === 0) {
    return (
      <EmptyMessage>
        {search ? m.no_matching_threads() : m.no_threads()}
      </EmptyMessage>
    )
  }

  return (
    <ul className='flex min-w-0 flex-1 flex-col gap-1.5 overflow-y-auto p-2'>
      {visible.map((thread) => (
        <li key={thread.id}>
          <ThreadCard
            thread={thread}
            selected={thread.id === selectedThreadId}
            onSelect={selectThread}
          />
        </li>
      ))}
    </ul>
  )
}

function EmptyMessage({ children }: { children: string }) {
  return (
    <p className='text-muted-foreground flex-1 p-4 text-center text-xs'>
      {children}
    </p>
  )
}
