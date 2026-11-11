import { withCleanup } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { invoke } from '@tauri-apps/api/core'
import { Input } from '@hoshi/ui/components/input'
import { Button } from '@hoshi/ui/components/button'
import { openUrl } from '@tauri-apps/plugin-opener'
import type { ForgeKind } from '@/bindings/ForgeKind'
import type { ForgeAccount } from '@/bindings/ForgeAccount'
import type { CommandError } from '@/bindings/CommandError'
import { type FormEvent, useCallback, useEffect, useId, useState } from 'react'
import { SettingsPage, SettingsRow, SettingsSection } from './SettingsLayout'
import { forgeErrorMessage } from '@/components/panel/pull-request/use-pull-request-actions'

const BITBUCKET_TOKENS_URL =
  'https://id.atlassian.com/manage-profile/security/api-tokens'

const BITBUCKET_SCOPES = [
  'read:user:bitbucket',
  'read:repository:bitbucket',
  'read:pullrequest:bitbucket',
  'write:pullrequest:bitbucket',
  'read:workspace:bitbucket'
]

export function SourceControlSection() {
  const [accounts, setAccounts] = useState<ForgeAccount[] | null>(null)

  const load = useCallback(() => {
    invoke<ForgeAccount[]>('get_forge_accounts')
      .then(setAccounts)
      .catch((error) => console.error('hoshi: forge accounts failed', error))
  }, [])

  useEffect(load, [load])

  const login = (forge: ForgeKind) =>
    accounts?.find((account) => account.forge === forge)?.login ?? null

  return (
    <SettingsPage>
      <SettingsSection title={m.settings_forges()}>
        <SettingsRow
          title={m.github()}
          description={m.github_description()}
          control={
            <span className='text-muted-foreground text-xs'>
              {accounts === null
                ? m.forge_checking()
                : login('github')
                  ? m.forge_signed_in({ login: login('github') ?? '' })
                  : m.forge_signed_out()}
            </span>
          }
        />
        <BitbucketRow
          checking={accounts === null}
          login={login('bitbucket')}
          onChange={(account) =>
            setAccounts((current) => [
              ...(current ?? []).filter((each) => each.forge !== account.forge),
              account
            ])
          }
        />
      </SettingsSection>
    </SettingsPage>
  )
}

function connectErrorMessage(error: CommandError): string {
  switch (error.code) {
    case 'forgeUnauthorized':
      return m.bitbucket_error_credentials()
    case 'forgeScopesMissing':
      return m.bitbucket_error_scopes({ scopes: error.scopes.join(', ') })
    default:
      return forgeErrorMessage(error)
  }
}

function BitbucketRow({
  checking,
  login,
  onChange
}: {
  checking: boolean
  login: string | null
  onChange: (account: ForgeAccount) => void
}) {
  const [connecting, setConnecting] = useState(false)
  const [email, setEmail] = useState('')
  const [token, setToken] = useState('')
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const id = useId()

  const close = () => {
    setConnecting(false)
    setToken('')
    setError(null)
  }

  const connect = async (event: FormEvent) => {
    event.preventDefault()
    setPending(true)
    setError(null)
    return await withCleanup(
      async () => {
        try {
          onChange(
            await invoke<ForgeAccount>('connect_bitbucket', { email, token })
          )
          close()
        } catch (failure) {
          setError(connectErrorMessage(failure as CommandError))
        }
      },
      () => {
        setPending(false)
      }
    )
  }

  const disconnect = async () => {
    setPending(true)
    return await withCleanup(
      async () => {
        try {
          await invoke('disconnect_forge', { forge: 'bitbucket' })
          onChange({ forge: 'bitbucket', login: null })
        } catch (failure) {
          setError(forgeErrorMessage(failure as CommandError))
        }
      },
      () => {
        setPending(false)
      }
    )
  }

  return (
    <SettingsRow
      title={m.bitbucket()}
      description={m.bitbucket_description()}
      control={
        checking ? (
          <span className='text-muted-foreground text-xs'>
            {m.forge_checking()}
          </span>
        ) : login ? (
          <>
            <span className='text-muted-foreground text-xs'>
              {m.forge_signed_in({ login })}
            </span>
            <Button
              variant='outline'
              size='sm'
              disabled={pending}
              onClick={() => void disconnect()}
            >
              {m.forge_disconnect()}
            </Button>
          </>
        ) : connecting ? null : (
          <Button
            variant='outline'
            size='sm'
            onClick={() => setConnecting(true)}
          >
            {m.forge_connect()}
          </Button>
        )
      }
    >
      {connecting && !login ? (
        <form
          onSubmit={(event) => void connect(event)}
          className='border-border mt-3 flex flex-col gap-3 rounded-lg border p-3'
        >
          <p className='text-muted-foreground text-xs text-pretty'>
            {m.bitbucket_token_help()}{' '}
            <button
              type='button'
              onClick={() => void openUrl(BITBUCKET_TOKENS_URL)}
              className='text-foreground cursor-pointer underline underline-offset-2'
            >
              {m.bitbucket_create_token()}
            </button>
          </p>
          <div className='flex flex-wrap gap-1'>
            {BITBUCKET_SCOPES.map((scope) => (
              <code
                key={scope}
                className='bg-muted text-muted-foreground rounded px-1.5 py-0.5 text-[0.6875rem]'
              >
                {scope}
              </code>
            ))}
          </div>
          <div className='flex flex-col gap-1'>
            <label htmlFor={`${id}-email`} className='text-xs font-medium'>
              {m.bitbucket_email()}
            </label>
            <Input
              id={`${id}-email`}
              type='email'
              required
              autoComplete='email'
              value={email}
              onChange={(event) => setEmail(event.currentTarget.value)}
              placeholder='you@example.com'
            />
          </div>
          <div className='flex flex-col gap-1'>
            <label htmlFor={`${id}-token`} className='text-xs font-medium'>
              {m.bitbucket_api_token()}
            </label>
            <Input
              id={`${id}-token`}
              type='password'
              required
              autoComplete='off'
              value={token}
              onChange={(event) => setToken(event.currentTarget.value)}
            />
          </div>
          {error ? (
            <p role='alert' className='text-destructive text-xs'>
              {error}
            </p>
          ) : null}
          <div className='flex justify-end gap-2'>
            <Button type='button' variant='ghost' size='sm' onClick={close}>
              {m.cancel()}
            </Button>
            <Button
              type='submit'
              size='sm'
              disabled={pending || !email.trim() || !token.trim()}
            >
              {pending ? m.forge_checking() : m.forge_connect()}
            </Button>
          </div>
        </form>
      ) : null}
    </SettingsRow>
  )
}
