import type { ReactNode } from 'react'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { SettingsRow } from './SettingsLayout'
import { Loading03Icon } from '@hugeicons/core-free-icons'
import { Button } from '@hoshi/ui/components/button'
import { useProviderUpdateStore } from '@/state/provider-update-store'
import {
  ProviderUpdateStatus,
  useTurnsRunning,
  versionSummary
} from '@/components/updates/ProviderUpdateStatus'

export function ProviderVersionRow() {
  const version = useProviderUpdateStore((state) => state.version)
  const checking = useProviderUpdateStore((state) => state.checking)
  const check = useProviderUpdateStore((state) => state.check)
  const update = useProviderUpdateStore((state) => state.update)
  const turnsRunning = useTurnsRunning()
  const running = version?.update.status === 'running'
  const canUpdate =
    !running && version?.updateAvailable && version.updateCommand !== null

  let control: ReactNode
  if (running) {
    control = (
      <Button size='sm' variant='outline' disabled>
        <HugeiconsIcon
          icon={Loading03Icon}
          strokeWidth={2}
          className='animate-spin'
        />
        {m.provider_update_running()}
      </Button>
    )
  } else if (canUpdate && version.latest) {
    control = (
      <Button
        size='sm'
        disabled={turnsRunning}
        title={turnsRunning ? m.provider_update_busy() : undefined}
        onClick={() => void update()}
      >
        {m.provider_update_to({ version: version.latest })}
      </Button>
    )
  } else {
    control = (
      <Button
        size='sm'
        variant='outline'
        disabled={checking}
        onClick={() => void check()}
      >
        {checking ? (
          <HugeiconsIcon
            icon={Loading03Icon}
            strokeWidth={2}
            className='animate-spin'
          />
        ) : null}
        {m.provider_check_updates()}
      </Button>
    )
  }

  return (
    <SettingsRow
      title={m.provider_version()}
      description={
        version ? versionSummary(version) : m.provider_version_description()
      }
      control={control}
    >
      {version && version.update.status !== 'idle' ? (
        <div className='mt-3'>
          <ProviderUpdateStatus version={version} />
        </div>
      ) : null}
    </SettingsRow>
  )
}
