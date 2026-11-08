import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { useAppStore } from '@/state/app-store'
import type { ProviderVersion } from '@/bindings/ProviderVersion'
import type { ProviderUpdateFailure } from '@/bindings/ProviderUpdateFailure'
import {
  Alert02Icon,
  CheckmarkCircle02Icon,
  Loading03Icon
} from '@hugeicons/core-free-icons'

/** The CLI an update replaces is what turns run. */
export function useTurnsRunning() {
  return useAppStore((state) =>
    state.threads.some((thread) => thread.activeTurnId !== null)
  )
}

export function versionSummary(version: ProviderVersion) {
  const { installed, latest } = version
  if (!installed) return m.provider_version_not_found()
  if (version.updateAvailable && latest) {
    return m.provider_version_update_available({ installed, latest })
  }
  return m.provider_version_up_to_date({ installed })
}

function failureMessage(reason: ProviderUpdateFailure) {
  switch (reason.kind) {
    case 'turnsRunning':
      return m.provider_update_turns_running()
    case 'unsupported':
      return m.provider_update_unsupported()
    case 'spawnFailed':
      return m.provider_update_spawn_failed()
    case 'timedOut':
      return m.provider_update_timed_out()
    case 'exited':
      return reason.code === null
        ? m.provider_update_stopped()
        : m.provider_update_exited({ code: reason.code })
  }
}

/** How the last update goes or went; nothing while none ran. */
export function ProviderUpdateStatus({
  version
}: {
  version: ProviderVersion
}) {
  const { update } = version
  if (update.status === 'idle') {
    return version.updateAvailable && !version.updateCommand ? (
      <p className='text-muted-foreground text-xs text-pretty'>
        {m.provider_update_unsupported()}
      </p>
    ) : null
  }
  if (update.status === 'running') {
    return (
      <StatusLine icon={Loading03Icon} spin>
        {update.line ?? m.provider_update_running()}
      </StatusLine>
    )
  }
  if (update.status === 'succeeded') {
    return (
      <StatusLine icon={CheckmarkCircle02Icon} tone='text-status-done'>
        {m.provider_update_succeeded({ version: version.installed ?? '' })}
      </StatusLine>
    )
  }
  const message =
    update.status === 'unchanged'
      ? m.provider_update_unchanged()
      : failureMessage(update.reason)
  const output = update.status === 'failed' ? update.output : null
  return (
    <div className='flex min-w-0 flex-col gap-1.5'>
      <StatusLine icon={Alert02Icon} tone='text-status-error' wrap>
        {message}
      </StatusLine>
      {output ? (
        <pre className='bg-muted text-muted-foreground max-h-32 overflow-auto rounded-md px-2 py-1.5 font-mono text-[0.6875rem] leading-4 whitespace-pre-wrap'>
          {output}
        </pre>
      ) : null}
    </div>
  )
}

function StatusLine({
  icon,
  tone,
  spin = false,
  wrap = false,
  children
}: {
  icon: typeof Alert02Icon
  tone?: string
  spin?: boolean
  wrap?: boolean
  children: string
}) {
  return (
    <p className='text-muted-foreground flex min-w-0 items-start gap-1.5 text-xs'>
      <HugeiconsIcon
        icon={icon}
        strokeWidth={2}
        className={cn('mt-px size-3.5 shrink-0', tone, spin && 'animate-spin')}
      />
      <span className={cn('min-w-0', wrap ? 'text-pretty' : 'truncate')}>
        {children}
      </span>
    </p>
  )
}
