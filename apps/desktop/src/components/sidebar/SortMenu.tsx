import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { Button } from '@hoshi/ui/components/button'
import { FilterHorizontalIcon } from '@hugeicons/core-free-icons'
import { type ThreadSort, useSidebarStore } from '@/state/sidebar-store'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger
} from '@hoshi/ui/components/dropdown-menu'

const OPTIONS = [
  { value: 'activity', label: m.sort_by_activity },
  { value: 'created', label: m.sort_by_created }
] as const

export function SortMenu() {
  const threadSort = useSidebarStore((state) => state.threadSort)
  const setThreadSort = useSidebarStore((state) => state.setThreadSort)

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            variant='ghost'
            size='icon-sm'
            aria-label={m.filter_threads()}
          />
        }
      >
        <HugeiconsIcon icon={FilterHorizontalIcon} strokeWidth={2} />
      </DropdownMenuTrigger>
      <DropdownMenuContent align='end' className='w-44'>
        <DropdownMenuGroup>
          <DropdownMenuLabel>{m.sort_threads()}</DropdownMenuLabel>
          <DropdownMenuRadioGroup
            value={threadSort}
            onValueChange={(value: ThreadSort) => setThreadSort(value)}
          >
            {OPTIONS.map(({ value, label }) => (
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
