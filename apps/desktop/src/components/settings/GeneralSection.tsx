import { useEffect, useState } from 'react'
import { m } from '@/paraglide/messages'
import { homeDir } from '@tauri-apps/api/path'
import { open } from '@tauri-apps/plugin-dialog'
import { Input } from '@hoshi/ui/components/input'
import { Button } from '@hoshi/ui/components/button'
import { Switch } from '@hoshi/ui/components/switch'
import { useSettingsStore } from '@/state/settings-store'
import type { StartupView } from '@/bindings/StartupView'
import type { GeneralSettings } from '@/bindings/GeneralSettings'
import { SettingsPage, SettingsRow, SettingsSection } from './SettingsLayout'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue
} from '@hoshi/ui/components/select'

const INACTIVE_DAYS = [1, 3, 7, 14, 30]

const daysLabel = (count: number) =>
  count === 1 ? m.one_day() : m.days_count({ count })

export function GeneralSection() {
  const general = useSettingsStore((state) => state.settings?.general)
  const updateSettings = useSettingsStore((state) => state.updateSettings)
  const error = useSettingsStore((state) => state.error)

  if (!general) {
    return (
      <SettingsPage>
        {error === 'load' ? (
          <p className='text-destructive text-sm'>{m.settings_load_failed()}</p>
        ) : null}
      </SettingsPage>
    )
  }

  const update = (patch: Partial<GeneralSettings>) =>
    updateSettings({ general: { ...general, ...patch } })
  const startupItems: { value: StartupView; label: string }[] = [
    { value: 'inbox', label: m.inbox() },
    { value: 'lastProject', label: m.startup_last_project() }
  ]
  const days = INACTIVE_DAYS.includes(general.inactiveAfterDays)
    ? INACTIVE_DAYS
    : [...INACTIVE_DAYS, general.inactiveAfterDays].sort((a, b) => a - b)
  const dayItems = days.map((count) => ({
    value: String(count),
    label: daysLabel(count)
  }))

  return (
    <SettingsPage>
      <SettingsSection title={m.settings_general()} hideTitle>
        <SettingsRow
          title={m.startup_view()}
          description={m.startup_view_description()}
          htmlFor='settings-startup-view'
          control={
            <Select
              items={startupItems}
              value={general.startupView}
              onValueChange={(value: StartupView | null) => {
                if (value) update({ startupView: value })
              }}
            >
              <SelectTrigger id='settings-startup-view' className='min-w-40'>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {startupItems.map(({ value, label }) => (
                  <SelectItem key={value} value={value}>
                    {label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          }
        />
        <SettingsRow
          title={m.inactive_after()}
          description={m.inactive_after_description()}
          htmlFor='settings-inactive-after'
          control={
            <Select
              items={dayItems}
              value={String(general.inactiveAfterDays)}
              onValueChange={(value: string | null) => {
                if (value) update({ inactiveAfterDays: Number(value) })
              }}
            >
              <SelectTrigger id='settings-inactive-after' className='min-w-32'>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {dayItems.map(({ value, label }) => (
                  <SelectItem key={value} value={value}>
                    {label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          }
        />
        <SettingsRow
          title={m.projects_folder()}
          description={m.projects_folder_description()}
          htmlFor='settings-projects-folder'
          control={
            <ProjectsFolderField
              value={general.projectsFolder}
              onChange={(projectsFolder) => update({ projectsFolder })}
            />
          }
        />
        <SettingsRow
          title={m.notifications_setting()}
          description={m.notifications_setting_description()}
          htmlFor='settings-notifications'
          control={
            <Switch
              id='settings-notifications'
              checked={general.notifications}
              onCheckedChange={(notifications) => update({ notifications })}
            />
          }
        />
      </SettingsSection>
      {error === 'save' ? (
        <p className='text-destructive px-4 text-xs'>
          {m.settings_save_failed()}
        </p>
      ) : null}
    </SettingsPage>
  )
}

function ProjectsFolderField({
  value,
  onChange
}: {
  value: string | null
  onChange: (value: string | null) => void
}) {
  const [draft, setDraft] = useState(value ?? '')
  const [home, setHome] = useState('')

  useEffect(() => setDraft(value ?? ''), [value])
  useEffect(() => {
    void homeDir().then(setHome)
  }, [])

  const commit = (next: string) => {
    const trimmed = next.trim()
    if (trimmed !== (value ?? '')) onChange(trimmed || null)
  }

  const browse = async () => {
    const picked = await open({
      directory: true,
      multiple: false,
      defaultPath: value ?? home
    })
    if (picked) commit(picked)
  }

  return (
    <div className='flex items-center gap-2'>
      <Input
        id='settings-projects-folder'
        value={draft}
        placeholder={home}
        spellCheck={false}
        onChange={(event) => setDraft(event.currentTarget.value)}
        onBlur={() => commit(draft)}
        onKeyDown={(event) => {
          if (event.key === 'Enter') commit(draft)
        }}
        className='w-56'
      />
      <Button variant='outline' size='sm' onClick={() => void browse()}>
        {m.browse()}
      </Button>
    </div>
  )
}
