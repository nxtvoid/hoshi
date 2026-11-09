import { m } from '@/paraglide/messages'
import { ComputerIcon, Moon02Icon, Sun03Icon } from '@hugeicons/core-free-icons'

export const THEME_OPTIONS = [
  { value: 'system', label: m.theme_system, icon: ComputerIcon },
  { value: 'light', label: m.theme_light, icon: Sun03Icon },
  { value: 'dark', label: m.theme_dark, icon: Moon02Icon }
] as const
