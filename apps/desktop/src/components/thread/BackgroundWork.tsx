import { m } from '@/paraglide/messages'
import { formatElapsed } from '@hoshi/utils'
import { useAppStore } from '@/state/app-store'
import { HugeiconsIcon } from '@hugeicons/react'
import { Button } from '@hoshi/ui/components/button'
import { useStoppingStore } from '@/state/stopping-store'
import { Loading03Icon } from '@hugeicons/core-free-icons'
import { useEffect, useState, type ReactNode } from 'react'
import type { BackgroundTask } from '@/bindings/BackgroundTask'
import { useBackgroundTaskStore } from '@/state/background-task-store'

/**
 * The commands the agent left running in the background, at the end of the transcript, also
 * once its turn completed. While the agent only waits for them they replace `status`, the
 * running turn's; while it works beside them they follow it.
 */
export function BackgroundWork({ status }: { status: ReactNode }) {
  const threadId = useAppStore((state) => state.selectedThreadId)
  const background = useBackgroundTaskStore((state) =>
    threadId ? state.byThread[threadId] : undefined
  )
  const stopping = useStoppingStore((state) =>
    threadId ? threadId in state.since : false
  )
  if (!background || stopping) return status ? <li>{status}</li> : null
  return (
    <li className='flex flex-col gap-2'>
      {background.waiting ? null : status}
      {background.tasks.map((task) => (
        <BackgroundTaskLine
          key={task.taskId}
          task={task}
          waiting={background.waiting}
        />
      ))}
    </li>
  )
}

function BackgroundTaskLine({
  task,
  waiting
}: {
  task: BackgroundTask
  waiting: boolean
}) {
  const stopBackgroundTask = useAppStore((state) => state.stopBackgroundTask)
  const stopping = useBackgroundTaskStore(
    (state) => task.taskId in state.stopping
  )
  const [now, setNow] = useState(Date.now)
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(timer)
  }, [])

  return (
    <span className='text-muted-foreground flex min-w-0 items-center gap-2 text-sm'>
      <HugeiconsIcon
        icon={Loading03Icon}
        strokeWidth={2}
        className='size-4 shrink-0 animate-spin'
      />
      <span className='shrink-0'>
        {waiting ? m.background_waiting() : m.background_running()}
      </span>
      <span aria-hidden className='shrink-0'>
        ·
      </span>
      <span className='text-foreground/80 min-w-0 truncate'>
        {task.description || m.background_task()}
      </span>
      <span aria-hidden className='shrink-0'>
        ·
      </span>
      <span className='shrink-0 tabular-nums'>
        {formatElapsed(now - task.startedAt)}
      </span>
      <Button
        size='xs'
        variant='ghost'
        disabled={stopping}
        onClick={() => void stopBackgroundTask(task.taskId)}
        className='text-muted-foreground hover:text-foreground -ml-1 shrink-0'
      >
        {stopping ? m.background_task_stopping() : m.background_task_stop()}
      </Button>
    </span>
  )
}
