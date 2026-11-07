import type { ReactNode } from 'react'
import { Button } from '@hoshi/ui/components/button'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@hoshi/ui/components/tooltip'

type IconButtonProps = {
  label: string
  onClick?: () => void
  disabled?: boolean
  active?: boolean
  side?: 'top' | 'bottom'
  children: ReactNode
}

export function IconButton({
  label,
  onClick,
  disabled,
  active = false,
  side = 'bottom',
  children
}: IconButtonProps) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            variant={active ? 'secondary' : 'ghost'}
            size='icon-sm'
            aria-label={label}
            aria-current={active ? 'page' : undefined}
            disabled={disabled}
            onClick={onClick}
          />
        }
      >
        {children}
      </TooltipTrigger>
      <TooltipContent side={side}>{label}</TooltipContent>
    </Tooltip>
  )
}
