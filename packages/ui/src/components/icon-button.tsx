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
  className?: string
  children: ReactNode
}

export function IconButton({
  label,
  onClick,
  disabled,
  active = false,
  side = 'bottom',
  className,
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
            className={className}
          />
        }
      >
        {children}
      </TooltipTrigger>
      <TooltipContent side={side}>{label}</TooltipContent>
    </Tooltip>
  )
}
