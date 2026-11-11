import type * as React from 'react'
import { Combobox as ComboboxPrimitive } from '@base-ui/react/combobox'
import { cn } from 'cn'
import { HugeiconsIcon } from '@hugeicons/react'
import {
  Search01Icon,
  Tick02Icon,
  UnfoldMoreIcon
} from '@hugeicons/core-free-icons'

const Combobox = ComboboxPrimitive.Root

function ComboboxTrigger({
  className,
  children,
  size = 'default',
  ...props
}: ComboboxPrimitive.Trigger.Props & { size?: 'sm' | 'default' }) {
  return (
    <ComboboxPrimitive.Trigger
      data-slot='combobox-trigger'
      data-size={size}
      className={cn(
        'border-input focus-visible:border-ring focus-visible:ring-ring/50 dark:bg-input/30 dark:hover:bg-input/50 flex w-fit items-center justify-between gap-1.5 rounded-lg border bg-transparent py-2 pr-2 pl-2.5 text-sm whitespace-nowrap transition-colors outline-none select-none focus-visible:ring-3 disabled:cursor-not-allowed disabled:opacity-50 data-[size=default]:h-8 data-[size=sm]:h-7 data-[size=sm]:rounded-[min(var(--radius-md),10px)]',
        className
      )}
      {...props}
    >
      <span className='min-w-0 flex-1 truncate text-left'>{children}</span>
      <HugeiconsIcon
        icon={UnfoldMoreIcon}
        strokeWidth={2}
        className='text-muted-foreground pointer-events-none size-4 shrink-0'
      />
    </ComboboxPrimitive.Trigger>
  )
}

function ComboboxContent({
  className,
  side = 'bottom',
  sideOffset = 4,
  align = 'start',
  alignOffset = 0,
  ...props
}: ComboboxPrimitive.Popup.Props &
  Pick<
    ComboboxPrimitive.Positioner.Props,
    'align' | 'alignOffset' | 'side' | 'sideOffset'
  >) {
  return (
    <ComboboxPrimitive.Portal>
      <ComboboxPrimitive.Positioner
        side={side}
        sideOffset={sideOffset}
        align={align}
        alignOffset={alignOffset}
        className='isolate z-50'
      >
        <ComboboxPrimitive.Popup
          data-slot='combobox-content'
          className={cn(
            'bg-popover text-popover-foreground ring-foreground/10 flex max-h-(--available-height) w-72 max-w-(--available-width) min-w-(--anchor-width) origin-(--transform-origin) flex-col overflow-hidden rounded-lg shadow-md ring-1 transition-[opacity,scale] duration-150 ease-out data-ending-style:scale-98 data-ending-style:opacity-0 data-starting-style:scale-98 data-starting-style:opacity-0',
            className
          )}
          {...props}
        />
      </ComboboxPrimitive.Positioner>
    </ComboboxPrimitive.Portal>
  )
}

function ComboboxSearch({
  className,
  ...props
}: ComboboxPrimitive.Input.Props) {
  return (
    <div className='border-border/70 flex shrink-0 items-center gap-2 border-b px-2.5'>
      <HugeiconsIcon
        icon={Search01Icon}
        strokeWidth={2}
        className='text-muted-foreground pointer-events-none size-3.5 shrink-0'
      />
      <ComboboxPrimitive.Input
        data-slot='combobox-search'
        className={cn(
          'placeholder:text-muted-foreground h-9 w-full min-w-0 bg-transparent font-sans text-sm outline-none',
          className
        )}
        {...props}
      />
    </div>
  )
}

function ComboboxList({ className, ...props }: ComboboxPrimitive.List.Props) {
  return (
    <ComboboxPrimitive.List
      data-slot='combobox-list'
      className={cn('min-h-0 outline-none', className)}
      {...props}
    />
  )
}

function ComboboxItem({
  className,
  children,
  ...props
}: ComboboxPrimitive.Item.Props) {
  return (
    <ComboboxPrimitive.Item
      data-slot='combobox-item'
      className={cn(
        'data-highlighted:bg-accent data-highlighted:text-accent-foreground relative flex w-full cursor-default items-center gap-1.5 rounded-md py-1 pr-8 pl-1.5 text-sm outline-hidden select-none data-disabled:pointer-events-none data-disabled:opacity-50',
        className
      )}
      {...props}
    >
      {children}
      <ComboboxPrimitive.ItemIndicator
        render={
          <span className='pointer-events-none absolute right-2 flex size-4 items-center justify-center' />
        }
      >
        <HugeiconsIcon icon={Tick02Icon} strokeWidth={2} className='size-4' />
      </ComboboxPrimitive.ItemIndicator>
    </ComboboxPrimitive.Item>
  )
}

function ComboboxEmpty({ className, ...props }: ComboboxPrimitive.Empty.Props) {
  return (
    <ComboboxPrimitive.Empty
      data-slot='combobox-empty'
      className={cn(
        'text-muted-foreground px-2.5 text-center text-xs not-empty:py-3',
        className
      )}
      {...props}
    />
  )
}

function ComboboxStatus({
  className,
  ...props
}: React.ComponentProps<typeof ComboboxPrimitive.Status>) {
  return (
    <ComboboxPrimitive.Status
      data-slot='combobox-status'
      className={cn(
        'text-muted-foreground px-2.5 text-center text-xs not-empty:py-3',
        className
      )}
      {...props}
    />
  )
}

export {
  Combobox,
  ComboboxContent,
  ComboboxEmpty,
  ComboboxItem,
  ComboboxList,
  ComboboxSearch,
  ComboboxStatus,
  ComboboxTrigger
}
