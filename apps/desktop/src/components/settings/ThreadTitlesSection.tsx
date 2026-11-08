import { m } from '@/paraglide/messages'
import { Switch } from '@hoshi/ui/components/switch'
import { useModelsStore } from '@/state/models-store'
import { Textarea } from '@hoshi/ui/components/textarea'
import { useSettingsStore } from '@/state/settings-store'
import type { ThreadTitleSettings } from '@/bindings/ThreadTitleSettings'
import { SettingsRow, SettingsSection } from './SettingsLayout'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue
} from '@hoshi/ui/components/select'

/** The select's value for the provider's own quick model. */
const DEFAULT_MODEL = 'default'

/** Whether and how a small model names new threads. */
export function ThreadTitlesSection() {
  const titles = useSettingsStore((state) => state.settings?.threadTitles)
  const updateSettings = useSettingsStore((state) => state.updateSettings)
  const catalog = useModelsStore((state) => state.catalog)
  if (!titles) return null

  const update = (patch: Partial<ThreadTitleSettings>) =>
    updateSettings({ threadTitles: { ...titles, ...patch } })
  const models = [
    { value: DEFAULT_MODEL, label: m.thread_titles_model_default() },
    ...(catalog?.models ?? [])
      .filter((model) => !model.legacy || model.slug === titles.model)
      .map((model) => ({ value: model.slug, label: model.name }))
  ]

  return (
    <SettingsSection title={m.settings_thread_titles()}>
      <SettingsRow
        title={m.thread_titles_generate()}
        description={m.thread_titles_generate_description()}
        htmlFor='thread-titles-generate'
        control={
          <Switch
            id='thread-titles-generate'
            checked={titles.generate}
            onCheckedChange={(generate) => update({ generate })}
          />
        }
      />
      <SettingsRow
        title={m.thread_titles_model()}
        description={m.thread_titles_model_description()}
        htmlFor='thread-titles-model'
        control={
          <Select
            items={models}
            value={titles.model ?? DEFAULT_MODEL}
            disabled={!titles.generate}
            onValueChange={(value: string | null) =>
              update({
                model: !value || value === DEFAULT_MODEL ? null : value
              })
            }
          >
            <SelectTrigger id='thread-titles-model' className='min-w-48'>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {models.map(({ value, label }) => (
                <SelectItem key={value} value={value}>
                  {label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        }
      />
      <SettingsRow
        title={m.thread_titles_instructions()}
        description={m.thread_titles_instructions_description()}
        htmlFor='thread-titles-instructions'
      >
        <Textarea
          key={titles.instructions}
          id='thread-titles-instructions'
          className='mt-3 min-h-16 text-sm'
          defaultValue={titles.instructions}
          disabled={!titles.generate}
          placeholder={m.thread_titles_instructions_placeholder()}
          onBlur={(event) => {
            const instructions = event.currentTarget.value.trim()
            if (instructions !== titles.instructions) update({ instructions })
          }}
        />
      </SettingsRow>
    </SettingsSection>
  )
}
