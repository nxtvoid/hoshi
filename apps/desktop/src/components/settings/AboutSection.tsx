import { m } from '@/paraglide/messages'
import { useEffect, useState } from 'react'
import { getVersion } from '@tauri-apps/api/app'
import { LogoMark } from '@/components/brand/Logo'
import { SettingsPage, SettingsRow, SettingsSection } from './SettingsLayout'

export function AboutSection() {
  const [version, setVersion] = useState<string | null>(null)

  // The version comes from the Tauri runtime, outside React.
  useEffect(() => {
    let active = true
    getVersion().then((value) => {
      if (active) setVersion(value)
    })
    return () => {
      active = false
    }
  }, [])

  return (
    <SettingsPage>
      <SettingsSection title={m.settings_about()} hideTitle>
        <SettingsRow
          title={
            <span className='flex items-center gap-2'>
              <LogoMark className='size-5' />
              {m.app_name()}
            </span>
          }
          description={version ? m.app_version({ version }) : null}
        />
      </SettingsSection>
    </SettingsPage>
  )
}
