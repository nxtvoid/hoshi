import { type ReactNode, useRef } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import { ArrowDown01Icon } from '@hugeicons/core-free-icons'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger
} from '@hoshi/ui/components/dropdown-menu'

export const ROW =
  'flex h-8 w-full min-w-0 cursor-pointer items-center gap-2.5 rounded-lg px-2.5 text-left text-[13px] font-medium text-foreground/80 outline-none transition-colors hover:bg-foreground/5 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset disabled:cursor-default disabled:opacity-60 disabled:hover:bg-transparent'
export const ICON = 'size-4 shrink-0 text-muted-foreground'

export const SPLIT_PRIMARY =
  'flex h-8 min-w-0 flex-1 cursor-pointer items-center gap-2.5 rounded-s-lg px-2.5 text-left text-[13px] font-medium text-foreground/80 outline-none hover:bg-foreground/5 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset'

/** A row whose main part runs an action and whose arrow opens alternatives. */
export function SplitRow({
  primary,
  menu,
  menuLabel
}: {
  primary: ReactNode
  /** `null` when there is nothing to choose from. */
  menu: ReactNode
  menuLabel: string
}) {
  const row = useRef<HTMLDivElement>(null)

  return (
    <div
      ref={row}
      className='group/split hover:bg-foreground/[0.02] flex items-center rounded-lg'
    >
      {primary}
      {menu ? (
        <>
          <span className='group-hover/split:bg-border/70 h-4 w-px shrink-0 bg-transparent' />
          <DropdownMenu>
            <DropdownMenuTrigger
              render={
                <button
                  type='button'
                  aria-label={menuLabel}
                  className='hover:bg-foreground/5 focus-visible:ring-ring flex h-8 w-8 shrink-0 cursor-pointer items-center justify-center rounded-e-lg outline-none focus-visible:ring-2 focus-visible:ring-inset'
                />
              }
            >
              <HugeiconsIcon
                icon={ArrowDown01Icon}
                strokeWidth={2}
                className={ICON}
              />
            </DropdownMenuTrigger>
            <DropdownMenuContent anchor={row} align='end'>
              {menu}
            </DropdownMenuContent>
          </DropdownMenu>
        </>
      ) : null}
    </div>
  )
}
