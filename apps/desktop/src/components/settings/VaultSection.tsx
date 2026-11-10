import { useEffect, useRef, useState } from 'react'
import { cn, formatRelativeTime } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { getLocale } from '@/paraglide/runtime'
import { HugeiconsIcon } from '@hugeicons/react'
import { confirm } from '@/state/dialog-store'
import { useAppStore } from '@/state/app-store'
import { Button } from '@hoshi/ui/components/button'
import { Switch } from '@hoshi/ui/components/switch'
import { accessChoices, effectiveAccess, fieldLabels } from '@/lib/vault'
import { useSettingsStore } from '@/state/settings-store'
import type { VaultSecret } from '@/bindings/VaultSecret'
import type { VaultAccess } from '@/bindings/VaultAccess'
import { IconButton } from '@hoshi/ui/components/icon-button'
import { copySecret, useVaultStore } from '@/state/vault-store'
import { VaultSecretDialog } from '@/components/vault/VaultSecretDialog'
import { SettingsPage, SettingsRow, SettingsSection } from './SettingsLayout'
import { ACCESS_LABELS } from '@/components/vault/vault-messages'
import {
  Add01Icon,
  Copy01Icon,
  Delete02Icon,
  Key01Icon,
  MoreHorizontalIcon,
  PencilEdit02Icon,
  SquareLock02Icon,
  Tick02Icon
} from '@hugeicons/core-free-icons'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger
} from '@hoshi/ui/components/dropdown-menu'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue
} from '@hoshi/ui/components/select'

const COPIED_MS = 1500

/** Editing a secret, or adding one. */
type Editing = { secret?: VaultSecret }

export function VaultSection() {
  const vault = useSettingsStore((state) => state.settings?.vault)
  const updateSettings = useSettingsStore((state) => state.updateSettings)
  const secrets = useVaultStore((state) => state.secrets)
  const projects = useAppStore((state) => state.projects)
  const [editing, setEditing] = useState<Editing | null>(null)

  useEffect(() => {
    void useVaultStore.getState().load()
  }, [])

  const alwaysAllow = vault?.alwaysAllow ?? false
  const projectTitle = (projectId: string | null) =>
    projectId === null
      ? m.vault_scope_all()
      : (projects.find((project) => project.id === projectId)?.title ??
        m.vault_scope_all())

  return (
    <SettingsPage>
      <SettingsSection title={m.settings_vault()} hideTitle>
        <SettingsRow
          title={m.vault_always_allow()}
          description={m.vault_always_allow_description()}
          htmlFor='settings-vault-always-allow'
          control={
            <Switch
              id='settings-vault-always-allow'
              checked={alwaysAllow}
              disabled={!vault}
              onCheckedChange={(next) =>
                void updateSettings({ vault: { alwaysAllow: next } })
              }
            />
          }
        />
      </SettingsSection>
      <section className='flex flex-col gap-2.5'>
        <div className='flex items-center justify-between gap-3 px-4'>
          <h2 className='text-base font-semibold'>{m.vault_secrets()}</h2>
          <Button size='sm' variant='outline' onClick={() => setEditing({})}>
            <HugeiconsIcon icon={Add01Icon} strokeWidth={2} />
            {m.vault_add()}
          </Button>
        </div>
        <div className='border-border/60 bg-card/40 [&>*+*]:border-border/50 rounded-xl border [&>*+*]:border-t'>
          {secrets === null ? null : secrets.length === 0 ? (
            <div className='flex flex-col items-center gap-1 px-6 py-10 text-center'>
              <HugeiconsIcon
                icon={Key01Icon}
                strokeWidth={2}
                className='text-muted-foreground mb-1 size-5'
              />
              <p className='text-sm font-medium'>{m.vault_empty()}</p>
              <p className='text-muted-foreground max-w-sm text-xs text-pretty'>
                {m.vault_empty_description()}
              </p>
            </div>
          ) : (
            secrets.map((secret) => (
              <SecretRow
                key={secret.id}
                secret={secret}
                scope={projectTitle(secret.projectId)}
                alwaysAllow={alwaysAllow}
                onEdit={() => setEditing({ secret })}
              />
            ))
          )}
        </div>
        <p className='text-muted-foreground flex items-center gap-1.5 px-4 text-xs'>
          <HugeiconsIcon
            icon={SquareLock02Icon}
            strokeWidth={2}
            className='size-3.5 shrink-0'
          />
          {m.vault_storage_note()}
        </p>
      </section>
      {editing ? (
        <VaultSecretDialog
          secret={editing.secret}
          onClose={() => setEditing(null)}
        />
      ) : null}
    </SettingsPage>
  )
}

function SecretRow({
  secret,
  scope,
  alwaysAllow,
  onEdit
}: {
  secret: VaultSecret
  scope: string
  alwaysAllow: boolean
  onEdit: () => void
}) {
  const save = useVaultStore((state) => state.save)
  const remove = useVaultStore((state) => state.remove)
  const [copied, setCopied] = useState(false)
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current)
    },
    []
  )

  const copy = async (fieldId: string) => {
    try {
      await copySecret(secret.id, fieldId)
      if (timer.current) clearTimeout(timer.current)
      setCopied(true)
      timer.current = setTimeout(() => setCopied(false), COPIED_MS)
    } catch (error) {
      console.error('hoshi: failed to copy a secret', error)
    }
  }

  const setAccess = (access: VaultAccess) =>
    save(secret.id, {
      name: secret.name,
      description: secret.description,
      projectId: secret.projectId,
      access,
      fields: secret.fields.map((field) => ({ ...field, value: null }))
    }).catch((error) => console.error('hoshi: failed to save a secret', error))

  const erase = async () => {
    const confirmed = await confirm({
      title: m.vault_delete_confirm_title({ name: secret.name }),
      description: m.vault_delete_confirm_description(),
      confirmLabel: m.vault_delete(),
      tone: 'destructive'
    })
    if (!confirmed) return
    await remove(secret.id).catch((error) =>
      console.error('hoshi: failed to delete a secret', error)
    )
  }

  const accessItems = accessChoices(alwaysAllow).map((access) => ({
    value: access,
    label: ACCESS_LABELS[access]()
  }))
  const shown = effectiveAccess(secret.access, alwaysAllow)
  const labels = fieldLabels(secret)
  const details = [
    scope,
    labels.length > 0 ? labels.join(', ') : null,
    secret.lastUsedAt
      ? m.vault_last_used({
          time: formatRelativeTime(secret.lastUsedAt, new Date(), getLocale())
        })
      : null
  ].filter((detail) => detail !== null)

  return (
    <div className='flex items-center gap-3 px-4 py-2.5'>
      <div className='flex min-w-0 flex-1 flex-col gap-0.5'>
        <span className='truncate text-sm font-medium'>{secret.name}</span>
        <span className='text-muted-foreground truncate text-xs'>
          {secret.description
            ? `${secret.description} · ${details.join(' · ')}`
            : details.join(' · ')}
        </span>
      </div>
      <Select
        items={accessItems}
        value={shown}
        onValueChange={(next: VaultAccess | null) => {
          if (next && next !== shown) void setAccess(next)
        }}
      >
        <SelectTrigger
          size='sm'
          aria-label={m.vault_access_label()}
          className={cn(
            'min-w-28 text-xs',
            shown === 'hidden' && 'text-muted-foreground'
          )}
        >
          <SelectValue />
        </SelectTrigger>
        <SelectContent align='end'>
          {accessItems.map((item) => (
            <SelectItem key={item.value} value={item.value}>
              {item.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      {secret.fields.length === 1 ? (
        <IconButton
          label={copied ? m.copied() : m.vault_copy()}
          onClick={() => void copy(secret.fields[0].id)}
          className='text-muted-foreground'
        >
          <HugeiconsIcon
            icon={copied ? Tick02Icon : Copy01Icon}
            strokeWidth={2}
          />
        </IconButton>
      ) : (
        <DropdownMenu>
          <DropdownMenuTrigger
            render={
              <Button
                variant='ghost'
                size='icon-sm'
                aria-label={copied ? m.copied() : m.vault_copy()}
                className='text-muted-foreground'
              />
            }
          >
            <HugeiconsIcon
              icon={copied ? Tick02Icon : Copy01Icon}
              strokeWidth={2}
            />
          </DropdownMenuTrigger>
          <DropdownMenuContent align='end' className='w-48'>
            {secret.fields.map((field) => (
              <DropdownMenuItem
                key={field.id}
                onClick={() => void copy(field.id)}
              >
                <HugeiconsIcon icon={Copy01Icon} strokeWidth={2} />
                <span className='min-w-0 flex-1 truncate'>
                  {m.vault_copy_field({ field: field.label })}
                </span>
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      )}
      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <Button
              variant='ghost'
              size='icon-sm'
              aria-label={m.vault_secret_actions({ name: secret.name })}
              className='text-muted-foreground'
            />
          }
        >
          <HugeiconsIcon icon={MoreHorizontalIcon} strokeWidth={2} />
        </DropdownMenuTrigger>
        <DropdownMenuContent align='end' className='w-40'>
          <DropdownMenuItem onClick={onEdit}>
            <HugeiconsIcon icon={PencilEdit02Icon} strokeWidth={2} />
            {m.vault_edit()}
          </DropdownMenuItem>
          <DropdownMenuItem variant='destructive' onClick={() => void erase()}>
            <HugeiconsIcon icon={Delete02Icon} strokeWidth={2} />
            {m.vault_delete()}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  )
}
