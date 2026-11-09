import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { type ThemePreference, useThemeStore } from '@/state/theme-store'
import { SettingsPage, SettingsRow, SettingsSection } from './SettingsLayout'
import { ComputerIcon, Moon02Icon, Sun03Icon } from '@hugeicons/core-free-icons'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue
} from '@hoshi/ui/components/select'

export const THEME_OPTIONS = [
  { value: 'system', label: m.theme_system, icon: ComputerIcon },
  { value: 'light', label: m.theme_light, icon: Sun03Icon },
  { value: 'dark', label: m.theme_dark, icon: Moon02Icon }
] as const

export function AppearanceSection() {
  const preference = useThemeStore((state) => state.preference)
  const setPreference = useThemeStore((state) => state.setPreference)
  const items = THEME_OPTIONS.map(({ value, label }) => ({
    value,
    label: label()
  }))

  return (
    <SettingsPage>
      <SettingsSection title={m.settings_appearance()} hideTitle>
        <SettingsRow
          title={m.theme()}
          description={m.theme_description()}
          htmlFor='settings-theme'
          control={
            <Select
              items={items}
              value={preference}
              onValueChange={(value: ThemePreference | null) => {
                if (value) setPreference(value)
              }}
            >
              <SelectTrigger id='settings-theme' className='min-w-40'>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {THEME_OPTIONS.map(({ value, label, icon }) => (
                  <SelectItem key={value} value={value}>
                    <HugeiconsIcon icon={icon} strokeWidth={2} />
                    {label()}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          }
        />
      </SettingsSection>
    </SettingsPage>
  )
}
