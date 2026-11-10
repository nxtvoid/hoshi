import { type FormEvent, useEffect, useId, useRef, useState } from 'react'
import { m } from '@/paraglide/messages'
import { VAULT_ACCESSES } from '@/lib/vault'
import { HugeiconsIcon } from '@hugeicons/react'
import { useAppStore } from '@/state/app-store'
import { useVaultStore } from '@/state/vault-store'
import { Input } from '@hoshi/ui/components/input'
import { Button } from '@hoshi/ui/components/button'
import type { VaultSecret } from '@/bindings/VaultSecret'
import type { VaultAccess } from '@/bindings/VaultAccess'
import type { CommandError } from '@/bindings/CommandError'
import type { VaultSecretInput } from '@/bindings/VaultSecretInput'
import {
  ACCESS_DESCRIPTIONS,
  ACCESS_LABELS,
  vaultErrorMessage
} from './vault-messages'
import {
  Add01Icon,
  Cancel01Icon,
  ViewIcon,
  ViewOffIcon
} from '@hugeicons/core-free-icons'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogTitle
} from '@hoshi/ui/components/dialog'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue
} from '@hoshi/ui/components/select'

/** Stands for every project in the project picker, whose values are project ids. */
const ALL_PROJECTS = '*'

const LABEL = 'text-muted-foreground text-xs'

const MAX_FIELDS = 6

/** A value of the secret as the dialog edits it. */
type FieldDraft = {
  key: string
  /** The stored field it changes; `null` for one being added. */
  id: string | null
  label: string
  masked: boolean
  value: string
}

function newField(): FieldDraft {
  return {
    key: crypto.randomUUID(),
    id: null,
    label: '',
    masked: true,
    value: ''
  }
}

function draftsOf(secret: VaultSecret | undefined): FieldDraft[] {
  if (!secret) return [newField()]
  return secret.fields.map((field) => ({ ...newField(), ...field }))
}

type VaultSecretDialogProps = {
  /** The secret to change; absent to add one. */
  secret?: VaultSecret
  /** What a new secret starts with. */
  initial?: { name?: string; projectId?: string | null }
  /**
   * Where the value comes from when the user doesn't type it, such as a note: its first
   * characters, shown in place of the field.
   */
  valueHint?: string
  /** In place of "New secret" or "Edit secret". */
  title?: string
  /** Saves in place of the vault, as moving a value out of a note does. */
  onSubmit?: (secret: VaultSecretInput) => Promise<unknown>
  onClose: () => void
}

/** Adds a secret to the vault, or changes one. */
export function VaultSecretDialog({
  secret,
  initial,
  valueHint,
  title,
  onSubmit,
  onClose
}: VaultSecretDialogProps) {
  const nameId = useId()
  const valuesId = useId()
  const descriptionId = useId()
  const projectFieldId = useId()
  const accessId = useId()
  const projects = useAppStore((state) => state.projects)
  const save = useVaultStore((state) => state.save)
  const [name, setName] = useState(secret?.name ?? initial?.name ?? '')
  const [fields, setFields] = useState(() => draftsOf(secret))
  const [named, setNamed] = useState(
    () => fields.length > 1 || fields.some((field) => field.label !== '')
  )
  const [description, setDescription] = useState(secret?.description ?? '')
  const [projectId, setProjectId] = useState(
    secret ? secret.projectId : (initial?.projectId ?? null)
  )
  const [access, setAccess] = useState<VaultAccess>(secret?.access ?? 'ask')
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<CommandError | null>(null)

  const typesValue = valueHint === undefined
  const complete =
    name.trim() !== '' &&
    (!typesValue ||
      fields.every(
        (field) =>
          (field.id !== null || field.value.trim() !== '') &&
          (fields.length === 1 || field.label.trim() !== '')
      ))

  const change = (key: string, change: Partial<FieldDraft>) =>
    setFields((current) =>
      current.map((field) =>
        field.key === key ? { ...field, ...change } : field
      )
    )
  const valuesRef = useRef<HTMLFieldSetElement>(null)
  const namingNext = useRef(false)
  const addField = () => {
    setNamed(true)
    setFields((current) => [...current, newField()])
    namingNext.current = true
  }

  useEffect(() => {
    if (!namingNext.current) return
    namingNext.current = false
    const labels =
      valuesRef.current?.querySelectorAll<HTMLInputElement>(
        'input[data-field-label]'
      ) ?? []
    Array.from(labels)
      .find((label) => label.value.trim() === '')
      ?.focus()
  })

  const projectItems = [
    { value: ALL_PROJECTS, label: m.vault_scope_all() },
    ...projects.map((project) => ({ value: project.id, label: project.title }))
  ]
  const accessItems = VAULT_ACCESSES.map((option) => ({
    value: option,
    label: ACCESS_LABELS[option]()
  }))

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    if (!complete || saving) return
    const input: VaultSecretInput = {
      name,
      description,
      projectId,
      access,
      fields: typesValue
        ? fields.map((field) => ({
            id: field.id,
            label: field.label,
            masked: field.masked,
            value: field.value.trim() !== '' ? field.value : null
          }))
        : []
    }
    setSaving(true)
    setError(null)
    try {
      await (onSubmit ? onSubmit(input) : save(secret?.id ?? null, input))
      onClose()
    } catch (failure) {
      setError(failure as CommandError)
      setSaving(false)
    }
  }

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className='sm:max-w-md'>
        <form className='contents' onSubmit={(event) => void submit(event)}>
          <DialogTitle>
            {title ?? (secret ? m.vault_edit_title() : m.vault_new_title())}
          </DialogTitle>
          <DialogDescription>{m.vault_dialog_description()}</DialogDescription>
          <div className='flex flex-col gap-1'>
            <label htmlFor={nameId} className={LABEL}>
              {m.vault_field_name()}
            </label>
            <Input
              id={nameId}
              autoFocus
              autoComplete='off'
              value={name}
              placeholder={m.vault_field_name_placeholder()}
              onChange={(event) => setName(event.target.value)}
            />
          </div>
          {typesValue ? (
            <div className='flex flex-col gap-1'>
              <div className='flex items-center justify-between gap-2'>
                {named ? (
                  <span id={valuesId} className={LABEL}>
                    {m.vault_field_values()}
                  </span>
                ) : (
                  <label htmlFor={valuesId} className={LABEL}>
                    {m.vault_field_value()}
                  </label>
                )}
                {fields.length < MAX_FIELDS ? (
                  <Button
                    type='button'
                    size='xs'
                    variant='ghost'
                    className='text-muted-foreground -my-1 -mr-1.5'
                    onClick={addField}
                  >
                    <HugeiconsIcon icon={Add01Icon} strokeWidth={2} />
                    {m.vault_add_field()}
                  </Button>
                ) : null}
              </div>
              {named ? (
                <fieldset
                  ref={valuesRef}
                  aria-labelledby={valuesId}
                  className='flex flex-col gap-1.5'
                >
                  {fields.map((field, index) => (
                    <div key={field.key} className='flex items-center gap-1.5'>
                      <Input
                        autoComplete='off'
                        value={field.label}
                        aria-label={m.vault_field_label()}
                        data-field-label
                        placeholder={
                          index === 0
                            ? m.vault_field_label_placeholder()
                            : undefined
                        }
                        className='w-32 shrink-0'
                        onChange={(event) =>
                          change(field.key, { label: event.target.value })
                        }
                      />
                      <FieldValue
                        field={field}
                        label={field.label || m.vault_field_value()}
                        placeholder={
                          field.id === null
                            ? undefined
                            : m.vault_field_value_unchanged()
                        }
                        onChange={(next) => change(field.key, next)}
                      />
                      {fields.length > 1 ? (
                        <Button
                          type='button'
                          size='icon-sm'
                          variant='ghost'
                          aria-label={m.vault_remove_field()}
                          className='text-muted-foreground'
                          onClick={() =>
                            setFields((current) =>
                              current.filter((each) => each.key !== field.key)
                            )
                          }
                        >
                          <HugeiconsIcon icon={Cancel01Icon} strokeWidth={2} />
                        </Button>
                      ) : null}
                    </div>
                  ))}
                </fieldset>
              ) : (
                fields.map((field) => (
                  <FieldValue
                    key={field.key}
                    id={valuesId}
                    field={field}
                    placeholder={
                      field.id === null ? undefined : m.vault_field_value_keep()
                    }
                    onChange={(next) => change(field.key, next)}
                  />
                ))
              )}
            </div>
          ) : (
            <p className='text-muted-foreground text-xs'>
              {m.vault_value_from_note({ hint: valueHint })}
            </p>
          )}
          <div className='flex flex-col gap-1'>
            <label htmlFor={descriptionId} className={LABEL}>
              {m.vault_field_description()}
            </label>
            <Input
              id={descriptionId}
              autoComplete='off'
              value={description}
              placeholder={m.vault_field_description_placeholder()}
              onChange={(event) => setDescription(event.target.value)}
            />
            <p className='text-muted-foreground text-[0.6875rem]'>
              {m.vault_field_description_hint()}
            </p>
          </div>
          <div className='grid grid-cols-2 gap-3'>
            <div className='flex min-w-0 flex-col gap-1'>
              <label htmlFor={projectFieldId} className={LABEL}>
                {m.vault_field_project()}
              </label>
              <Select
                items={projectItems}
                value={projectId ?? ALL_PROJECTS}
                onValueChange={(next: string | null) =>
                  setProjectId(next === ALL_PROJECTS ? null : next)
                }
              >
                <SelectTrigger id={projectFieldId} className='w-full min-w-0'>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {projectItems.map((item) => (
                    <SelectItem key={item.value} value={item.value}>
                      {item.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className='flex min-w-0 flex-col gap-1'>
              <label htmlFor={accessId} className={LABEL}>
                {m.vault_access_label()}
              </label>
              <Select
                items={accessItems}
                value={access}
                onValueChange={(next: VaultAccess | null) => {
                  if (next) setAccess(next)
                }}
              >
                <SelectTrigger id={accessId} className='w-full min-w-0'>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {accessItems.map((item) => (
                    <SelectItem key={item.value} value={item.value}>
                      {item.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </div>
          <p className='text-muted-foreground -mt-2 text-[0.6875rem]'>
            {ACCESS_DESCRIPTIONS[access]()}
          </p>
          {error ? (
            <p role='alert' className='text-destructive text-xs'>
              {vaultErrorMessage(error)}
            </p>
          ) : null}
          <DialogFooter>
            <Button type='button' variant='outline' onClick={onClose}>
              {m.cancel()}
            </Button>
            <Button type='submit' disabled={!complete || saving}>
              {m.save()}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}

/** The input of one value, hidden as a password is until the user shows it. */
function FieldValue({
  id,
  field,
  label,
  placeholder,
  onChange
}: {
  id?: string
  field: FieldDraft
  label?: string
  placeholder?: string
  onChange: (change: Partial<FieldDraft>) => void
}) {
  return (
    <div className='relative min-w-0 flex-1'>
      <Input
        id={id}
        type={field.masked ? 'password' : 'text'}
        autoComplete='off'
        spellCheck={false}
        value={field.value}
        aria-label={label}
        placeholder={placeholder}
        className='pr-9 font-mono'
        onChange={(event) => onChange({ value: event.target.value })}
      />
      <Button
        type='button'
        size='icon-xs'
        variant='ghost'
        aria-label={field.masked ? m.vault_show_value() : m.vault_hide_value()}
        aria-pressed={!field.masked}
        className='text-muted-foreground absolute top-1 right-1'
        onClick={() => onChange({ masked: !field.masked })}
      >
        <HugeiconsIcon
          icon={field.masked ? ViewIcon : ViewOffIcon}
          strokeWidth={2}
        />
      </Button>
    </div>
  )
}
