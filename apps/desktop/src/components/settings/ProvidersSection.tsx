import { m } from '@/paraglide/messages'
import { Input } from '@hoshi/ui/components/input'
import { Switch } from '@hoshi/ui/components/switch'
import { useSettingsStore } from '@/state/settings-store'
import { ThreadTitlesSection } from './ThreadTitlesSection'
import { ProviderVersionRow } from './ProviderVersionRow'
import type { SubagentEffort } from '@/bindings/SubagentEffort'
import { SettingsPage, SettingsRow, SettingsSection } from './SettingsLayout'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue
} from '@hoshi/ui/components/select'

export function ProvidersSection() {
  const claudeCode = useSettingsStore(
    (state) => state.settings?.providers.claudeCode
  )
  const planModeEnabled = useSettingsStore(
    (state) => state.settings?.planModeEnabled ?? false
  )
  const error = useSettingsStore((state) => state.error)
  const updateClaudeCode = useSettingsStore((state) => state.updateClaudeCode)
  const updateSettings = useSettingsStore((state) => state.updateSettings)

  if (!claudeCode) {
    return (
      <SettingsPage>
        {error === 'load' ? (
          <p className='text-destructive text-sm'>{m.settings_load_failed()}</p>
        ) : null}
      </SettingsPage>
    )
  }

  const subagentEffortItems: {
    value: SubagentEffort
    label: string
    description: string
  }[] = [
    {
      value: 'thread',
      label: m.subagent_effort_thread(),
      description: m.subagent_effort_thread_description()
    },
    {
      value: 'agentDecides',
      label: m.subagent_effort_agent_decides(),
      description: m.subagent_effort_agent_decides_description()
    }
  ]

  const commitBinaryPath = (value: string) => {
    const binaryPath = value.trim() || null
    if (binaryPath !== claudeCode.binaryPath) updateClaudeCode({ binaryPath })
  }

  return (
    <SettingsPage>
      <SettingsSection title={m.claude_code()}>
        <SettingsRow
          title={m.provider_enabled()}
          description={m.claude_code_enabled_description()}
          htmlFor='claude-code-enabled'
          control={
            <Switch
              id='claude-code-enabled'
              checked={claudeCode.enabled}
              onCheckedChange={(enabled) => updateClaudeCode({ enabled })}
            />
          }
        />
        <SettingsRow
          title={m.binary_path()}
          description={m.claude_code_binary_path_description()}
          htmlFor='claude-code-binary-path'
          control={
            <Input
              key={claudeCode.binaryPath ?? ''}
              id='claude-code-binary-path'
              className='w-64'
              defaultValue={claudeCode.binaryPath ?? ''}
              placeholder={m.claude_code_binary_placeholder()}
              spellCheck={false}
              autoComplete='off'
              onBlur={(event) => commitBinaryPath(event.currentTarget.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter') event.currentTarget.blur()
              }}
            />
          }
        />
        <SettingsRow
          title={m.subagent_effort()}
          description={m.subagent_effort_description()}
          htmlFor='claude-code-subagent-effort'
          control={
            <Select
              items={subagentEffortItems}
              value={claudeCode.subagentEffort}
              onValueChange={(value: SubagentEffort | null) => {
                if (value) updateClaudeCode({ subagentEffort: value })
              }}
            >
              <SelectTrigger
                id='claude-code-subagent-effort'
                className='min-w-40'
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent align='end' className='w-72'>
                {subagentEffortItems.map(({ value, label, description }) => (
                  <SelectItem key={value} value={value} className='py-1.5'>
                    <div className='flex min-w-0 flex-col gap-0.5'>
                      <span>{label}</span>
                      <span className='text-xs whitespace-normal opacity-70'>
                        {description}
                      </span>
                    </div>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          }
        />
        {claudeCode.enabled ? <ProviderVersionRow /> : null}
      </SettingsSection>
      <SettingsSection title={m.settings_composer()}>
        <SettingsRow
          title={m.plan_mode_setting()}
          description={m.plan_mode_setting_description()}
          htmlFor='plan-mode-enabled'
          control={
            <Switch
              id='plan-mode-enabled'
              checked={planModeEnabled}
              onCheckedChange={(enabled) =>
                updateSettings({ planModeEnabled: enabled })
              }
            />
          }
        />
      </SettingsSection>
      <ThreadTitlesSection />
      {error === 'save' ? (
        <p className='text-destructive px-4 text-xs'>
          {m.settings_save_failed()}
        </p>
      ) : null}
    </SettingsPage>
  )
}
