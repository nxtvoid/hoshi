import { cn } from '@hoshi/utils'
import { useEffect } from 'react'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { Button } from '@hoshi/ui/components/button'
import { metaOf } from '@/components/brand/provider-meta'
import { isWindows } from '@/components/brand/EditorIcon'
import { CommandBlock } from '@hoshi/ui/components/command-block'
import type { ProviderAccountStatus } from '@/bindings/ProviderAccountStatus'
import { useProviderUpdateStore } from '@/state/provider-update-store'
import { useProviderAccountStore } from '@/state/provider-account-store'
import { WizardDescription, WizardTitle } from '@hoshi/ui/components/wizard'
import {
  Loading03Icon,
  RefreshIcon,
  Tick02Icon
} from '@hugeicons/core-free-icons'

const INSTALL_COMMAND = isWindows()
  ? 'irm https://claude.ai/install.ps1 | iex'
  : 'curl -fsSL https://claude.ai/install.sh | bash'
const SIGN_IN_COMMAND = 'claude auth login'

export function AgentStep() {
  const status = useProviderAccountStore((state) => state.status)
  const checking = useProviderAccountStore((state) => state.checking)
  const check = useProviderAccountStore((state) => state.check)
  const version = useProviderUpdateStore(
    (state) => state.version?.installed ?? null
  )
  const meta = metaOf('claudeCode')

  useEffect(() => {
    if (useProviderAccountStore.getState().status !== 'ready') void check()
  }, [check])

  const command =
    status === 'notInstalled'
      ? INSTALL_COMMAND
      : status === 'signedOut'
        ? SIGN_IN_COMMAND
        : null

  return (
    <>
      <WizardTitle>{m.onboarding_agent_title()}</WizardTitle>
      <WizardDescription>{m.onboarding_agent_description()}</WizardDescription>
      <div className='bg-background mt-6 flex items-center gap-3 rounded-xl border px-4 py-3.5'>
        <span
          className={cn(
            'grid size-9 shrink-0 place-items-center rounded-lg',
            meta.tile
          )}
        >
          <meta.icon className='size-4.5' />
        </span>
        <div className='min-w-0 flex-1'>
          <span className='block text-sm font-medium'>{meta.label()}</span>
          <span
            role='status'
            className='text-muted-foreground mt-0.5 block truncate text-xs'
          >
            {detail(status, version)}
          </span>
        </div>
        <AgentBadge
          status={status}
          checking={checking}
          onCheck={() => void check()}
        />
      </div>
      {command ? (
        <div className='mt-4 flex flex-col gap-2.5'>
          <p className='text-muted-foreground text-sm'>
            {status === 'notInstalled'
              ? m.onboarding_agent_install_hint()
              : m.onboarding_agent_sign_in_hint()}
          </p>
          <CommandBlock
            command={command}
            copyLabel={m.onboarding_copy_command()}
            copiedLabel={m.copied()}
          />
          <Button
            variant='outline'
            size='sm'
            className='self-start'
            disabled={checking}
            onClick={() => void check()}
          >
            <HugeiconsIcon
              icon={RefreshIcon}
              strokeWidth={2}
              className={cn(checking && 'animate-spin')}
            />
            {m.onboarding_check_again()}
          </Button>
        </div>
      ) : null}
    </>
  )
}

function AgentBadge({
  status,
  checking,
  onCheck
}: {
  status: ProviderAccountStatus | null
  checking: boolean
  onCheck: () => void
}) {
  if (status === null) {
    return (
      <HugeiconsIcon
        icon={Loading03Icon}
        strokeWidth={2}
        className='text-muted-foreground size-4 shrink-0 animate-spin'
      />
    )
  }
  if (status === 'ready') {
    return (
      <span className='text-status-done flex shrink-0 items-center gap-1 text-xs font-medium'>
        <HugeiconsIcon icon={Tick02Icon} strokeWidth={2} className='size-3.5' />
        {m.onboarding_agent_ready()}
      </span>
    )
  }
  if (status === 'unknown') {
    return (
      <Button variant='ghost' size='xs' disabled={checking} onClick={onCheck}>
        <HugeiconsIcon
          icon={RefreshIcon}
          strokeWidth={2}
          className={cn(checking && 'animate-spin')}
        />
        {m.onboarding_check_again()}
      </Button>
    )
  }
  return null
}

function detail(status: ProviderAccountStatus | null, version: string | null) {
  switch (status) {
    case null:
      return m.onboarding_agent_checking()
    case 'ready':
      return version
        ? m.onboarding_agent_ready_version({ version })
        : m.onboarding_agent_ready_detail()
    case 'notInstalled':
      return m.onboarding_agent_not_installed()
    case 'signedOut':
      return m.onboarding_agent_signed_out()
    case 'unknown':
      return m.onboarding_agent_unknown()
  }
}
