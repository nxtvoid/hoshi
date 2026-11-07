import { useMemo } from 'react'
import { m } from '@/paraglide/messages'
import { ThreadCard } from './ThreadCard'
import { useAppStore } from '@/state/app-store'

export function ThreadList() {
  const threads = useAppStore((state) => state.threads)
  const search = useAppStore((state) => state.search)
  const projectId = useAppStore((state) => state.selectedProjectId)
  const selectedThreadId = useAppStore((state) => state.selectedThreadId)
  const selectThread = useAppStore((state) => state.selectThread)

  const visible = useMemo(() => {
    const query = search.trim().toLowerCase()
    return threads
      .filter(
        (thread) =>
          thread.projectId === projectId &&
          (query === '' || thread.title.toLowerCase().includes(query))
      )
      .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt))
  }, [threads, search, projectId])

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
