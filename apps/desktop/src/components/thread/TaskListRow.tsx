import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import type { Task } from '@/bindings/Task'
import type { TaskStatus } from '@/bindings/TaskStatus'
import {
  CheckmarkCircle02Icon,
  CircleIcon,
  Loading03Icon,
  TaskDone01Icon
} from '@hugeicons/core-free-icons'

const STATUS_ICONS: Record<TaskStatus, typeof CircleIcon> = {
  pending: CircleIcon,
  inProgress: Loading03Icon,
  completed: CheckmarkCircle02Icon
}

/** The agent's task list, kept up to date in place as it works through it. */
export function TaskListRow({
  tasks,
  working
}: {
  tasks: Task[]
  working: boolean
}) {
  const done = tasks.filter((task) => task.status === 'completed').length
  return (
    <div className='flex flex-col gap-1'>
      <div className='text-muted-foreground flex min-h-6 items-center gap-2 text-sm'>
        <HugeiconsIcon
          icon={TaskDone01Icon}
          strokeWidth={2}
          className='size-4 shrink-0'
        />
        <span className='text-foreground font-medium'>
          {m.task_list_progress({ done, total: tasks.length })}
        </span>
      </div>
      <ol className='border-border ms-2 flex flex-col gap-0.5 border-s ps-4 text-sm'>
        {tasks.map((task, index) => {
          const active = task.status === 'inProgress' && working
          return (
            <li
              key={`${index}:${task.subject}`}
              className={cn(
                'flex items-start gap-2',
                task.status === 'completed' && 'text-muted-foreground'
              )}
            >
              <HugeiconsIcon
                icon={
                  task.status === 'inProgress' && !working
                    ? CircleIcon
                    : STATUS_ICONS[task.status]
                }
                strokeWidth={2}
                className={cn(
                  'mt-0.5 size-3.5 shrink-0',
                  active && 'text-status-working animate-spin'
                )}
              />
              <span
                className={cn(
                  task.status === 'completed' && 'line-through',
                  active && 'text-foreground font-medium'
                )}
              >
                {active && task.activeForm ? task.activeForm : task.subject}
              </span>
            </li>
          )
        })}
      </ol>
    </div>
  )
}
