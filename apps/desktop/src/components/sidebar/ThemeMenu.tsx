import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { Button } from '@hoshi/ui/components/button'
import { type ThemePreference, useThemeStore } from '@/state/theme-store'
import { ComputerIcon, Moon02Icon, Sun03Icon } from '@hugeicons/core-free-icons'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger
} from '@hoshi/ui/components/dropdown-menu'

const OPTIONS = [
  { value: 'system', label: m.theme_system, icon: ComputerIcon },
  { value: 'light', label: m.theme_light, icon: Sun03Icon },
  { value: 'dark', label: m.theme_dark, icon: Moon02Icon }
] as const

export function ThemeMenu() {
  const preference = useThemeStore((state) => state.preference)
  const setPreference = useThemeStore((state) => state.setPreference)
  const current = OPTIONS.find((option) => option.value === preference)
  const currentIcon = current?.icon ?? ComputerIcon

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button variant='ghost' size='icon-sm' aria-label={m.theme()} />
        }
      >
        <HugeiconsIcon icon={currentIcon} strokeWidth={2} />
      </DropdownMenuTrigger>
      <DropdownMenuContent align='end' className='w-36'>
        <DropdownMenuRadioGroup
          value={preference}
          onValueChange={(value: ThemePreference) => setPreference(value)}
        >
          {OPTIONS.map(({ value, label, icon }) => (
            <DropdownMenuRadioItem key={value} value={value}>
              <HugeiconsIcon icon={icon} strokeWidth={2} />
              {label()}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
