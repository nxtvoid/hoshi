import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { Button } from '@hoshi/ui/components/button'
import { FilterHorizontalIcon } from '@hugeicons/core-free-icons'
import {
  type ThreadFilter,
  type ThreadSort,
  useSidebarStore
} from '@/state/sidebar-store'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger
} from '@hoshi/ui/components/dropdown-menu'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@hoshi/ui/components/tooltip'

const SORTS = [
  { value: 'activity', label: m.sort_by_activity },
  { value: 'created', label: m.sort_by_created }
] as const

const FILTERS = [
  { value: 'all', label: m.filter_all_threads },
  { value: 'worktrees', label: m.filter_worktrees },
  { value: 'pullRequests', label: m.filter_with_pull_request }
] as const

export function SortMenu() {
  const threadSort = useSidebarStore((state) => state.threadSort)
  const setThreadSort = useSidebarStore((state) => state.setThreadSort)
  const filter = useSidebarStore((state) => state.filter)
  const setFilter = useSidebarStore((state) => state.setFilter)

  return (
    <DropdownMenu>
      <Tooltip>
        <TooltipTrigger
          render={
            <DropdownMenuTrigger
              render={
                <Button
                  variant='ghost'
                  size='icon-sm'
                  aria-label={m.filter_threads()}
                  className={cn(filter !== 'all' && 'text-sidebar-primary')}
                />
              }
            />
          }
        >
          <HugeiconsIcon icon={FilterHorizontalIcon} strokeWidth={2} />
        </TooltipTrigger>
        <TooltipContent side='bottom'>{m.filter_threads()}</TooltipContent>
      </Tooltip>
      <DropdownMenuContent align='end' className='w-44'>
        <DropdownMenuGroup>
          <DropdownMenuLabel>{m.sort_threads()}</DropdownMenuLabel>
          <DropdownMenuRadioGroup
            value={threadSort}
            onValueChange={(value: ThreadSort) => setThreadSort(value)}
          >
            {SORTS.map(({ value, label }) => (
              <DropdownMenuRadioItem key={value} value={value}>
                {label()}
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuGroup>
        <DropdownMenuSeparator />
        <DropdownMenuGroup>
          <DropdownMenuLabel>{m.show_threads()}</DropdownMenuLabel>
          <DropdownMenuRadioGroup
            value={filter}
            onValueChange={(value: ThreadFilter) => setFilter(value)}
          >
            {FILTERS.map(({ value, label }) => (
              <DropdownMenuRadioItem key={value} value={value}>
                {label()}
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
