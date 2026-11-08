import type { ReactNode } from 'react'
import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { useModelsStore } from '@/state/models-store'
import { Button } from '@hoshi/ui/components/button'
import { useAppUpdateStore } from '@/state/app-update-store'
import { metaOf } from '@/components/brand/provider-meta'
import { useProviderUpdateStore } from '@/state/provider-update-store'
import type { ProviderVersion } from '@/bindings/ProviderVersion'
import {
  appBadge,
  type AppUpdate,
  providerBadge,
  strongestBadge
} from '@/lib/updates'
import {
  Popover,
  PopoverContent,
  PopoverTrigger
} from '@hoshi/ui/components/popover'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@hoshi/ui/components/tooltip'
import {
  Alert02Icon,
  CheckmarkCircle02Icon,
  Download04Icon,
  Loading03Icon
} from '@hugeicons/core-free-icons'
import {
  ProviderUpdateStatus,
  useTurnsRunning,
  versionSummary
} from './ProviderUpdateStatus'
import { LogoMark } from '@/components/brand/Logo'

/** Shows only while an update is available, running, or just ended. */
export function UpdatesButton() {
  const version = useProviderUpdateStore((state) => state.version)
  const seen = useProviderUpdateStore((state) => state.seen)
  const markSeen = useProviderUpdateStore((state) => state.markSeen)
  const appUpdate = useAppUpdateStore((state) => state.update)
  const badge = strongestBadge(
    appBadge(appUpdate),
    providerBadge(version, seen)
  )
  if (!badge) return null

  const icon =
    badge === 'running'
      ? Loading03Icon
      : badge === 'done'
        ? CheckmarkCircle02Icon
        : Download04Icon

  return (
    <Popover
      onOpenChange={(open) => {
        if (!open) markSeen()
      }}
    >
      <Tooltip>
        <TooltipTrigger
          render={
            <PopoverTrigger
              render={
                <Button
                  variant='ghost'
                  size='icon-sm'
                  aria-label={m.updates()}
                  className='relative'
                />
              }
            />
          }
        >
          <HugeiconsIcon
            icon={icon}
            strokeWidth={2}
            className={cn(
              badge === 'running' && 'animate-spin',
              badge === 'done' && 'text-status-done'
            )}
          />
          {badge === 'available' || badge === 'failed' ? (
            <span
              aria-hidden
              className={cn(
                'ring-sidebar absolute top-1 right-1 size-1.5 rounded-full ring-2',
                badge === 'failed' ? 'bg-status-error' : 'bg-foreground'
              )}
            />
          ) : null}
        </TooltipTrigger>
        <TooltipContent side='top'>{m.updates()}</TooltipContent>
      </Tooltip>
      <PopoverContent side='top' align='end' className='w-80 gap-3'>
        {appUpdate.status === 'idle' ? null : (
          <AppUpdateDetails update={appUpdate} />
        )}
        {version ? <ProviderUpdateDetails version={version} /> : null}
      </PopoverContent>
    </Popover>
  )
}

function AppUpdateDetails({ update }: { update: AppUpdate }) {
  const currentVersion = useAppUpdateStore((state) => state.currentVersion)
  const install = useAppUpdateStore((state) => state.install)
  const downloading = update.status === 'downloading'

  return (
    <div className='flex flex-col gap-2.5'>
      <UpdateRow
        icon={
          <span className='bg-muted flex size-7 shrink-0 items-center justify-center rounded-md'>
            <LogoMark className='size-3.5' />
          </span>
        }
        title={m.app_name()}
        summary={
          currentVersion
            ? m.provider_version_change({
                installed: currentVersion,
                latest: 'version' in update ? update.version : currentVersion
              })
            : null
        }
        action={
          downloading ? null : (
            <Button size='sm' variant='outline' onClick={() => void install()}>
              {update.status === 'failed'
                ? m.provider_update_retry()
                : m.app_update_install()}
            </Button>
          )
        }
      />
      {update.status === 'downloading' ? (
        <p className='text-muted-foreground flex items-center gap-1.5 text-xs'>
          <HugeiconsIcon
            icon={Loading03Icon}
            strokeWidth={2}
            className='size-3.5 shrink-0 animate-spin'
          />
          {update.percent === null
            ? m.app_update_downloading()
            : m.app_update_downloading_percent({ percent: update.percent })}
        </p>
      ) : update.status === 'failed' ? (
        <p className='text-muted-foreground flex items-start gap-1.5 text-xs'>
          <HugeiconsIcon
            icon={Alert02Icon}
            strokeWidth={2}
            className='text-status-error mt-px size-3.5 shrink-0'
          />
          {m.app_update_failed()}
        </p>
      ) : (
        <p className='text-muted-foreground text-xs'>
          {m.app_update_restarts()}
        </p>
      )}
    </div>
  )
}

function ProviderUpdateDetails({ version }: { version: ProviderVersion }) {
  const provider = useModelsStore((state) => state.catalog?.provider)
  const update = useProviderUpdateStore((state) => state.update)
  const turnsRunning = useTurnsRunning()
  const meta = metaOf(provider)
  const { installed, latest } = version
  const status = version.update.status
  const canUpdate =
    version.updateAvailable &&
    version.updateCommand !== null &&
    status !== 'running'

  return (
    <div className='flex flex-col gap-2.5'>
      <UpdateRow
        icon={
          <span
            className={cn(
              'flex size-7 shrink-0 items-center justify-center rounded-md',
              meta.tile
            )}
          >
            <meta.icon className='size-3.5' />
          </span>
        }
        title={meta.label()}
        summary={
          status === 'running' && installed && latest
            ? m.provider_version_change({ installed, latest })
            : versionSummary(version)
        }
        action={
          canUpdate ? (
            <Button
              size='sm'
              variant='outline'
              disabled={turnsRunning}
              onClick={() => void update()}
            >
              {status === 'failed' || status === 'unchanged'
                ? m.provider_update_retry()
                : m.provider_update()}
            </Button>
          ) : null
        }
      />
      <ProviderUpdateStatus version={version} />
      {canUpdate ? (
        <p className='text-muted-foreground text-xs'>
          {turnsRunning
            ? m.provider_update_busy()
            : m.provider_update_runs({ command: version.updateCommand ?? '' })}
        </p>
      ) : null}
    </div>
  )
}

function UpdateRow({
  icon,
  title,
  summary,
  action
}: {
  icon: ReactNode
  title: string
  summary: string | null
  action: ReactNode
}) {
  return (
    <div className='flex items-center gap-2.5'>
      {icon}
      <div className='flex min-w-0 flex-1 flex-col'>
        <span className='truncate text-sm font-medium'>{title}</span>
        {summary ? (
          <span className='text-muted-foreground truncate text-xs tabular-nums'>
            {summary}
          </span>
        ) : null}
      </div>
      {action}
    </div>
  )
}
