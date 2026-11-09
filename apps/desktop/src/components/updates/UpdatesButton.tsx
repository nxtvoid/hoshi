import type { ReactNode } from 'react'
import { m } from '@/paraglide/messages'
import { getLocale } from '@/paraglide/runtime'
import { HugeiconsIcon } from '@hugeicons/react'
import { openUrl } from '@tauri-apps/plugin-opener'
import { cn, formatRelativeTime } from '@hoshi/utils'
import { useModelsStore } from '@/state/models-store'
import { Button } from '@hoshi/ui/components/button'
import { metaOf } from '@/components/brand/provider-meta'
import { ProgressRing } from '@hoshi/ui/components/progress-ring'
import { useProviderUpdateStore } from '@/state/provider-update-store'
import type { ProviderVersion } from '@/bindings/ProviderVersion'
import {
  APP_UPDATES_ENABLED,
  useAppUpdateStore
} from '@/state/app-update-store'
import {
  type AppUpdate,
  providerBadge,
  releaseChanges,
  releaseCompareUrl,
  releaseNotesKey,
  releaseNotesTarget,
  type UpdateBadge,
  type UpdateButton,
  updateButton
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
  LinkSquare02Icon,
  Loading03Icon,
  RefreshIcon,
  RotateClockwiseIcon
} from '@hugeicons/core-free-icons'
import {
  ProviderUpdateStatus,
  useTurnsRunning,
  versionSummary
} from './ProviderUpdateStatus'
import { LogoMark } from '@/components/brand/Logo'

/** Checks hoshi's release feed, or opens what an update found or is doing. */
export function UpdatesButton() {
  const version = useProviderUpdateStore((state) => state.version)
  const seen = useProviderUpdateStore((state) => state.seen)
  const appUpdate = useAppUpdateStore((state) => state.update)
  const button = updateButton(
    appUpdate,
    providerBadge(version, seen),
    APP_UPDATES_ENABLED
  )
  if (button.kind === 'details') {
    return (
      <UpdateDetailsButton
        badge={button.badge}
        appUpdate={appUpdate}
        version={version}
      />
    )
  }
  return <CheckButton button={button} />
}

function CheckButton({
  button
}: {
  button: Exclude<UpdateButton, { kind: 'details' }>
}) {
  const check = useAppUpdateStore((state) => state.check)
  const off = button.kind === 'off'
  const checking = button.kind === 'check' && button.checking
  const label =
    button.kind === 'off'
      ? m.app_update_off_in_dev()
      : button.kind === 'upToDate'
        ? m.app_update_up_to_date({ version: button.version })
        : button.kind === 'checkFailed'
          ? m.app_update_check_failed({ error: button.message })
          : checking
            ? m.app_update_checking()
            : m.check_for_updates()

  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            variant='ghost'
            size='icon-sm'
            aria-label={label}
            disabled={off}
            focusableWhenDisabled
            onClick={() => {
              if (!off && !checking) void check(true)
            }}
            className='data-disabled:text-muted-foreground relative data-disabled:hover:bg-transparent'
          />
        }
      >
        <HugeiconsIcon
          icon={
            button.kind === 'upToDate' ? CheckmarkCircle02Icon : RefreshIcon
          }
          strokeWidth={2}
          className={cn(
            checking && 'animate-spin',
            button.kind === 'upToDate' && 'text-status-done'
          )}
        />
        {button.kind === 'checkFailed' ? (
          <Dot className='bg-status-error' />
        ) : null}
      </TooltipTrigger>
      <TooltipContent side='top' className='max-w-72'>
        {label}
      </TooltipContent>
    </Tooltip>
  )
}

function Dot({ className }: { className: string }) {
  return (
    <span
      aria-hidden
      className={cn(
        'ring-sidebar absolute top-1 right-1 size-1.5 rounded-full ring-2',
        className
      )}
    />
  )
}

function UpdateDetailsButton({
  badge,
  appUpdate,
  version
}: {
  badge: NonNullable<UpdateBadge>
  appUpdate: AppUpdate
  version: ProviderVersion | null
}) {
  const markSeen = useProviderUpdateStore((state) => state.markSeen)
  const loadReleaseNotes = useAppUpdateStore((state) => state.loadReleaseNotes)
  const downloading = appUpdate.status === 'downloading' ? appUpdate : null
  const label = downloading
    ? downloading.percent === null
      ? m.app_update_downloading()
      : m.app_update_downloading_percent({ percent: downloading.percent })
    : appUpdate.status === 'ready'
      ? m.app_update_restart()
      : badge === 'available' && appUpdate.status === 'available'
        ? m.app_update_to({ version: appUpdate.version })
        : m.updates()

  return (
    <Popover
      onOpenChange={(open) => {
        if (open) void loadReleaseNotes()
        else markSeen()
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
                  aria-label={label}
                  className='relative'
                />
              }
            />
          }
        >
          {downloading ? (
            <ProgressRing value={downloading.percent} />
          ) : (
            <HugeiconsIcon
              icon={BADGE_ICON[badge]}
              strokeWidth={2}
              className={cn(
                badge === 'running' && 'animate-spin',
                badge === 'done' && 'text-status-done'
              )}
            />
          )}
          {badge === 'available' || badge === 'ready' ? (
            <Dot className='bg-foreground' />
          ) : badge === 'failed' ? (
            <Dot className='bg-status-error' />
          ) : null}
        </TooltipTrigger>
        <TooltipContent side='top'>{label}</TooltipContent>
      </Tooltip>
      <PopoverContent side='top' align='end' className='w-80 gap-3'>
        {appUpdateShown(appUpdate) ? (
          <AppUpdateDetails update={appUpdate} />
        ) : null}
        {version ? <ProviderUpdateDetails version={version} /> : null}
      </PopoverContent>
    </Popover>
  )
}

const BADGE_ICON = {
  running: Loading03Icon,
  ready: RotateClockwiseIcon,
  failed: Download04Icon,
  done: CheckmarkCircle02Icon,
  available: Download04Icon
} satisfies Record<NonNullable<UpdateBadge>, unknown>

function appUpdateShown(update: AppUpdate) {
  return (
    update.status === 'available' ||
    update.status === 'downloading' ||
    update.status === 'ready' ||
    update.status === 'failed'
  )
}

function AppUpdateDetails({ update }: { update: AppUpdate }) {
  const currentVersion = useAppUpdateStore((state) => state.currentVersion)
  const download = useAppUpdateStore((state) => state.download)
  const restart = useAppUpdateStore((state) => state.restart)

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
          update.status === 'ready' ? (
            <Button size='sm' variant='outline' onClick={() => void restart()}>
              {m.app_update_restart()}
            </Button>
          ) : update.status === 'available' || update.status === 'failed' ? (
            <Button size='sm' variant='outline' onClick={() => void download()}>
              {update.status === 'failed'
                ? m.provider_update_retry()
                : m.app_update_download()}
            </Button>
          ) : null
        }
      />
      {update.status === 'downloading' ? (
        <p className='text-muted-foreground flex items-center gap-1.5 text-xs'>
          <ProgressRing value={update.percent} className='size-3.5 shrink-0' />
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
      {currentVersion ? (
        <ReleaseNotes installed={currentVersion} update={update} />
      ) : null}
    </div>
  )
}

function ReleaseNotes({
  installed,
  update
}: {
  installed: string
  update: AppUpdate
}) {
  const available = releaseNotesTarget(update)
  const releases = useAppUpdateStore((state) =>
    available
      ? state.releaseNotes[releaseNotesKey(installed, available)]
      : undefined
  )
  if (!available || !releases) return null
  const shown = releases.map((release) => ({
    ...release,
    changes: releaseChanges(release.body)
  }))
  const count = shown.reduce(
    (total, release) => total + release.changes.length,
    0
  )
  if (count === 0) return null
  const now = new Date()

  return (
    <div className='flex flex-col gap-2 border-t pt-2.5'>
      <div className='flex max-h-62 flex-col gap-3 overflow-y-auto'>
        {shown.map((release) => (
          <section key={release.version} className='flex flex-col gap-1'>
            <header className='flex items-baseline justify-between gap-2 text-xs'>
              <span className='font-semibold tabular-nums'>
                {release.version}
              </span>
              <span className='text-muted-foreground shrink-0'>
                {formatRelativeTime(release.publishedAt, now, getLocale())}
              </span>
            </header>
            <ul className='flex flex-col gap-0.5'>
              {release.changes.map((change) => (
                <li
                  key={change}
                  className='text-muted-foreground flex gap-1.5 text-xs'
                >
                  <span
                    aria-hidden
                    className='bg-muted-foreground/50 mt-1.5 size-1 shrink-0 rounded-full'
                  />
                  <span className='min-w-0'>{change}</span>
                </li>
              ))}
            </ul>
          </section>
        ))}
      </div>
      <div className='text-muted-foreground flex items-center justify-between gap-2 text-xs'>
        <span className='tabular-nums'>{m.app_update_changes({ count })}</span>
        <button
          type='button'
          onClick={() => void openUrl(releaseCompareUrl(installed, available))}
          className='hover:text-foreground focus-visible:ring-ring flex items-center gap-1 rounded outline-none focus-visible:ring-2'
        >
          {m.view_on_github()}
          <HugeiconsIcon
            icon={LinkSquare02Icon}
            strokeWidth={2}
            className='size-3 shrink-0'
          />
        </button>
      </div>
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
