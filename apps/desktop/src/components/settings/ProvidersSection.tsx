import { m } from '@/paraglide/messages'
import { Input } from '@hoshi/ui/components/input'
import { Switch } from '@hoshi/ui/components/switch'
import { useSettingsStore } from '@/state/settings-store'
import { SettingsPage, SettingsRow, SettingsSection } from './SettingsLayout'

export function ProvidersSection() {
  const claudeCode = useSettingsStore(
    (state) => state.settings?.providers.claudeCode
  )
  const error = useSettingsStore((state) => state.error)
  const updateClaudeCode = useSettingsStore((state) => state.updateClaudeCode)

  if (!claudeCode) {
    return (
      <SettingsPage>
        {error === 'load' ? (
          <p className='text-destructive text-sm'>{m.settings_load_failed()}</p>
        ) : null}
      </SettingsPage>
    )
  }

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
      </SettingsSection>
      {error === 'save' ? (
        <p className='text-destructive px-4 text-xs'>
          {m.settings_save_failed()}
        </p>
      ) : null}
    </SettingsPage>
  )
}
