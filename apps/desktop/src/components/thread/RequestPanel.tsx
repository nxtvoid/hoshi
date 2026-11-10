import { useEffect, useId, useMemo, useState } from 'react'
import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { toolTarget } from '@/lib/tool-calls'
import { fieldLabels, secretsAnswering } from '@/lib/vault'
import { HugeiconsIcon } from '@hugeicons/react'
import { Kbd } from '@hoshi/ui/components/kbd'
import { Button } from '@hoshi/ui/components/button'
import { Switch } from '@hoshi/ui/components/switch'
import { Markdown } from '@/components/markdown/Markdown'
import { useAppStore } from '@/state/app-store'
import { isPlanningWithUser } from '@/lib/agents'
import { useRequestStore } from '@/state/request-store'
import { useDelegatedRequest, useRequestActions } from './use-request-actions'
import { invoke } from '@tauri-apps/api/core'
import { Input } from '@hoshi/ui/components/input'
import type { SecretAnswer } from '@/bindings/SecretAnswer'
import type { CommandError } from '@/bindings/CommandError'
import { ensureVaultLoaded, useVaultStore } from '@/state/vault-store'
import { vaultErrorMessage } from '@/components/vault/vault-messages'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger
} from '@hoshi/ui/components/dropdown-menu'
import type {
  QuestionsContent,
  RequestItem,
  SecretRequestContent
} from '@/lib/requests'
import {
  ArrowRight01Icon,
  GitForkIcon,
  Key01Icon,
  ShieldUserIcon,
  TaskDone01Icon,
  UserQuestion01Icon
} from '@hugeicons/core-free-icons'

const PANEL =
  'border-border bg-card mb-2 flex flex-col gap-2.5 rounded-2xl border p-3 shadow-xs'

/**
 * What a delegated task waits on, answered from the thread that delegated it, with the
 * way to the task's own thread.
 */
export function DelegatedRequestPanel({
  threadId,
  cwd
}: {
  threadId: string
  cwd: string | null
}) {
  const delegated = useDelegatedRequest(threadId)
  const selectThread = useAppStore((state) => state.selectThread)
  if (!delegated) return null
  const { request, child } = delegated
  return (
    <div className='flex flex-col'>
      <div className='text-muted-foreground flex items-center gap-2 px-1 pb-1 text-xs'>
        <HugeiconsIcon
          icon={GitForkIcon}
          strokeWidth={2}
          className='size-3.5 shrink-0'
        />
        <span className='min-w-0 flex-1 truncate'>
          {m.delegated_request_from({ task: child.title })}
        </span>
        <Button
          size='xs'
          variant='ghost'
          onClick={() => selectThread(child.id)}
        >
          {m.delegation_open()}
          <HugeiconsIcon icon={ArrowRight01Icon} strokeWidth={2} />
        </Button>
      </div>
      <RequestPanel
        request={request}
        cwd={child.worktreePath ?? cwd}
        onKeepPlanning={() => selectThread(child.id)}
      />
    </div>
  )
}

/**
 * The tasks the thread delegated that wait for the user's changes to their plan, and whose
 * card is off screen, in one line above the composer: the oldest, and how many more.
 */
export function PlanningTasksNote({ threadId }: { threadId: string }) {
  const threads = useAppStore((state) => state.threads)
  const pendingTasks = useRequestStore((state) => state.pendingTasks)
  const onScreen = useRequestStore((state) => state.onScreenTasks)
  const selectThread = useAppStore((state) => state.selectThread)
  const sendTaskAnswer = useRequestStore((state) => state.sendTaskAnswer)
  const planning = useMemo(
    () =>
      threads
        .filter(
          (thread) =>
            thread.parentThreadId === threadId &&
            isPlanningWithUser(thread, pendingTasks) &&
            !onScreen[thread.id]
        )
        .sort((a, b) => a.createdAt.localeCompare(b.createdAt)),
    [threads, pendingTasks, onScreen, threadId]
  )
  const [task] = planning
  if (!task) return null
  return (
    <div className='text-muted-foreground flex items-center gap-2 px-1 pb-1 text-xs'>
      <HugeiconsIcon
        icon={GitForkIcon}
        strokeWidth={2}
        className='size-3.5 shrink-0'
      />
      <span className='min-w-0 flex-1 truncate'>
        {m.planning_task_note({ task: task.title })}
        {planning.length > 1
          ? ` ${m.planning_tasks_more({ count: planning.length - 1 })}`
          : null}
      </span>
      <Button
        size='xs'
        variant='ghost'
        className='text-muted-foreground'
        onClick={() => void sendTaskAnswer(task.id)}
      >
        {m.delegation_send_plan()}
      </Button>
      <Button size='xs' variant='ghost' onClick={() => selectThread(task.id)}>
        {m.delegation_open()}
        <HugeiconsIcon icon={ArrowRight01Icon} strokeWidth={2} />
      </Button>
    </div>
  )
}

/** What the agent waits on, above the composer, which takes typed answers and feedback. */
export function RequestPanel({
  request,
  cwd,
  onKeepPlanning
}: {
  request: RequestItem
  cwd: string | null
  /** Instead of sending the plan back at once: takes the user where they say what to change. */
  onKeepPlanning?: () => void
}) {
  const { content } = request
  switch (content.kind) {
    case 'approval':
      return <ApprovalPanel request={request} cwd={cwd} />
    case 'proposedPlan':
      return (
        <PlanPanel
          request={request}
          plan={content.plan}
          onKeepPlanning={onKeepPlanning}
        />
      )
    case 'questions':
      return <QuestionsPanel request={request} content={content} />
    case 'secretRequest':
      return <SecretPanel request={request} content={content} />
  }
}

/**
 * A secret the agent waits on. One the vault holds is approved here, one the user already
 * wrote in the chat is confirmed here, and any other is typed here, and goes to a file
 * and never to the chat.
 */
function SecretPanel({
  request,
  content
}: {
  request: RequestItem
  content: SecretRequestContent
}) {
  const [sending, setSending] = useState(false)
  const [error, setError] = useState<CommandError | null>(null)
  const answer = async (answer: SecretAnswer) => {
    setSending(true)
    setError(null)
    try {
      await invoke('answer_secret_request', {
        threadId: request.threadId,
        requestId: request.id,
        answer
      })
    } catch (failure) {
      console.error('hoshi: failed to answer a secret request', failure)
      setError(failure as CommandError)
      setSending(false)
    }
  }
  const failure = error ? (
    <p role='alert' className='text-destructive text-xs'>
      {vaultErrorMessage(error)}
    </p>
  ) : null

  if (content.vault) {
    return (
      <VaultSecretPanel
        vault={content.vault}
        reason={content.reason}
        sending={sending}
        failure={failure}
        onAnswer={(given) => void answer(given)}
      />
    )
  }
  if (content.pasted) {
    return (
      <PastedSecretPanel
        request={request}
        content={content}
        sending={sending}
        failure={failure}
        onAnswer={(given) => void answer(given)}
      />
    )
  }
  return (
    <TypedSecretPanel
      request={request}
      content={content}
      sending={sending}
      failure={failure}
      onAnswer={(given) => void answer(given)}
    />
  )
}

/** The project of the request's thread, where a kept secret goes; none for a chat. */
function useRequestProject(request: RequestItem) {
  const projectId = useAppStore(
    (state) =>
      state.threads.find((thread) => thread.id === request.threadId)
        ?.projectId ?? null
  )
  const projectTitle = useAppStore(
    (state) => state.projects.find((project) => project.id === projectId)?.title
  )
  return { projectId, projectTitle }
}

/** Asks whether to keep in the vault what the user already wrote in the chat. */
function PastedSecretPanel({
  request,
  content,
  sending,
  failure,
  onAnswer
}: {
  request: RequestItem
  content: SecretRequestContent
  sending: boolean
  failure: React.ReactNode
  onAnswer: (answer: SecretAnswer) => void
}) {
  const { projectTitle } = useRequestProject(request)
  const labels = content.fields.map((field) => field.label).join(', ')
  return (
    <div className={PANEL} role='alertdialog' aria-label={m.request_waiting()}>
      <PanelTitle icon={Key01Icon}>
        {m.secret_request_keep_title({ name: content.label })}
      </PanelTitle>
      <p className='text-muted-foreground text-xs'>{content.reason}</p>
      <p className='text-muted-foreground text-[0.6875rem]'>
        {labels ? `${labels} · ` : null}
        {m.secret_request_pasted_note()}
      </p>
      {failure}
      <div className='flex flex-wrap justify-end gap-1.5'>
        <Button
          size='sm'
          variant='ghost'
          disabled={sending}
          onClick={() => onAnswer({ type: 'decline' })}
        >
          {m.request_decline()}
        </Button>
        <Button
          size='sm'
          autoFocus
          disabled={sending}
          onClick={() =>
            onAnswer({
              type: 'keep',
              keep: { name: content.label, access: 'ask' }
            })
          }
        >
          {projectTitle
            ? m.secret_request_keep_project({ project: projectTitle })
            : m.secret_request_keep()}
        </Button>
      </div>
    </div>
  )
}

/** Asks to use a secret the vault holds: all of its values, or none. */
function VaultSecretPanel({
  vault,
  reason,
  sending,
  failure,
  onAnswer
}: {
  vault: NonNullable<SecretRequestContent['vault']>
  reason: string
  sending: boolean
  failure: React.ReactNode
  onAnswer: (answer: SecretAnswer) => void
}) {
  const { id, name } = vault
  const labels = useVaultStore((state) => {
    const secret = state.secrets?.find((secret) => secret.id === id)
    return secret ? fieldLabels(secret).join(', ') : ''
  })

  useEffect(ensureVaultLoaded, [])

  return (
    <div className={PANEL} role='alertdialog' aria-label={m.request_waiting()}>
      <PanelTitle icon={Key01Icon}>
        {m.secret_request_vault_title({ name })}
      </PanelTitle>
      <p className='text-muted-foreground text-xs'>{reason}</p>
      {labels ? (
        <p className='text-muted-foreground text-[0.6875rem]'>{labels}</p>
      ) : null}
      {failure}
      <div className='flex flex-wrap justify-end gap-1.5'>
        <Button
          size='sm'
          variant='ghost'
          disabled={sending}
          onClick={() => onAnswer({ type: 'decline' })}
        >
          {m.request_decline()}
        </Button>
        <Button
          size='sm'
          variant='outline'
          disabled={sending}
          onClick={() =>
            onAnswer({ type: 'vault', secretId: id, always: true })
          }
        >
          {m.secret_request_always_allow()}
        </Button>
        <Button
          size='sm'
          autoFocus
          disabled={sending}
          onClick={() =>
            onAnswer({ type: 'vault', secretId: id, always: false })
          }
        >
          {m.secret_request_allow_once()}
        </Button>
      </div>
    </div>
  )
}

function TypedSecretPanel({
  request,
  content,
  sending,
  failure,
  onAnswer
}: {
  request: RequestItem
  content: SecretRequestContent
  sending: boolean
  failure: React.ReactNode
  onAnswer: (answer: SecretAnswer) => void
}) {
  const keepId = useId()
  const fieldId = useId()
  const several = content.fields.length > 0
  const asked = several
    ? content.fields
    : [{ label: content.label, masked: true }]
  const [values, setValues] = useState(() => asked.map(() => ''))
  const complete = values.every((value) => value.trim() !== '')
  const [keep, setKeep] = useState(content.keepByDefault)
  const { projectId, projectTitle } = useRequestProject(request)
  const secrets = useVaultStore((state) => state.secrets)
  const saved = useMemo(
    () => secretsAnswering(secrets ?? [], projectId, asked.length),
    [secrets, projectId, asked.length]
  )

  useEffect(ensureVaultLoaded, [])

  return (
    <form
      className={PANEL}
      role='alertdialog'
      aria-label={m.request_waiting()}
      onSubmit={(event) => {
        event.preventDefault()
        if (!complete) return
        onAnswer({
          type: 'value',
          values,
          keep: keep ? { name: content.label, access: 'ask' } : null
        })
      }}
    >
      <PanelTitle icon={Key01Icon}>{content.label}</PanelTitle>
      <p className='text-muted-foreground text-xs'>{content.reason}</p>
      {asked.map((field, index) => (
        <div key={field.label} className='flex flex-col gap-1'>
          {several ? (
            <label
              htmlFor={`${fieldId}-${index}`}
              className='text-muted-foreground text-xs'
            >
              {field.label}
            </label>
          ) : null}
          <Input
            id={`${fieldId}-${index}`}
            type={field.masked ? 'password' : 'text'}
            autoFocus={index === 0}
            autoComplete='off'
            spellCheck={false}
            value={values[index]}
            placeholder={
              several
                ? undefined
                : (content.placeholder ?? m.secret_request_placeholder())
            }
            aria-label={several ? undefined : field.label}
            onChange={(event) =>
              setValues((current) =>
                current.map((value, at) =>
                  at === index ? event.target.value : value
                )
              )
            }
          />
        </div>
      ))}
      <p className='text-muted-foreground text-[0.6875rem]'>
        {several ? m.secret_request_note_fields() : m.secret_request_note()}
      </p>
      {failure}
      <div className='flex flex-wrap items-center gap-x-3 gap-y-1.5'>
        <label
          htmlFor={keepId}
          className='text-muted-foreground flex min-w-0 items-center gap-2 text-xs'
        >
          <Switch
            id={keepId}
            size='sm'
            checked={keep}
            onCheckedChange={setKeep}
          />
          <span className='truncate'>
            {projectTitle
              ? m.secret_request_keep_project({ project: projectTitle })
              : m.secret_request_keep()}
          </span>
        </label>
        <div className='ms-auto flex gap-1.5'>
          {saved.length > 0 ? (
            <DropdownMenu>
              <DropdownMenuTrigger
                render={
                  <Button
                    type='button'
                    size='sm'
                    variant='ghost'
                    disabled={sending}
                  />
                }
              >
                {m.secret_request_use_saved()}
              </DropdownMenuTrigger>
              <DropdownMenuContent align='end' className='w-64'>
                {saved.map((secret) => (
                  <DropdownMenuItem
                    key={secret.id}
                    onClick={() =>
                      onAnswer({
                        type: 'vault',
                        secretId: secret.id,
                        always: false
                      })
                    }
                  >
                    <HugeiconsIcon icon={Key01Icon} strokeWidth={2} />
                    <span className='min-w-0 flex-1 truncate'>
                      {secret.name}
                    </span>
                    {secret.projectId === null ? (
                      <span className='text-muted-foreground shrink-0 text-xs'>
                        {m.vault_scope_all()}
                      </span>
                    ) : null}
                  </DropdownMenuItem>
                ))}
              </DropdownMenuContent>
            </DropdownMenu>
          ) : null}
          <Button
            type='button'
            size='sm'
            variant='ghost'
            disabled={sending}
            onClick={() => onAnswer({ type: 'decline' })}
          >
            {m.request_decline()}
          </Button>
          <Button type='submit' size='sm' disabled={sending || !complete}>
            {m.secret_request_send()}
          </Button>
        </div>
      </div>
    </form>
  )
}

function PanelTitle({
  icon,
  children,
  aside
}: {
  icon: typeof ShieldUserIcon
  children: React.ReactNode
  aside?: React.ReactNode
}) {
  return (
    <div className='flex items-center gap-2 text-sm'>
      <HugeiconsIcon
        icon={icon}
        strokeWidth={2}
        className='text-status-waiting size-4 shrink-0'
      />
      <span className='text-foreground min-w-0 flex-1 font-medium'>
        {children}
      </span>
      {aside}
    </div>
  )
}

function ApprovalPanel({
  request,
  cwd
}: {
  request: RequestItem
  cwd: string | null
}) {
  const { decide } = useRequestActions(request)
  if (request.content.kind !== 'approval') return null
  const { toolName, input, reason, canAcceptForSession } = request.content
  const target = toolTarget(toolName, input, cwd) || input

  return (
    <div className={PANEL} role='alertdialog' aria-label={m.request_waiting()}>
      <PanelTitle icon={ShieldUserIcon}>
        {m.request_approval_title({ tool: toolName })}
      </PanelTitle>
      <pre className='bg-muted/40 text-foreground/85 max-h-32 overflow-auto rounded-md px-3 py-2 font-mono text-xs break-all whitespace-pre-wrap'>
        {target}
      </pre>
      {reason ? (
        <p className='text-muted-foreground text-xs'>{reason}</p>
      ) : null}
      <div className='flex flex-wrap justify-end gap-1.5'>
        <Button
          size='sm'
          variant='ghost'
          onClick={() => void decide('decline')}
        >
          {m.request_decline()}
        </Button>
        {canAcceptForSession ? (
          <Button
            size='sm'
            variant='outline'
            onClick={() => void decide('acceptForSession')}
          >
            {m.request_approve_session()}
          </Button>
        ) : null}
        <Button size='sm' onClick={() => void decide('accept')}>
          {m.request_approve()}
        </Button>
      </div>
    </div>
  )
}

function PlanPanel({
  request,
  plan,
  onKeepPlanning
}: {
  request: RequestItem
  plan: string
  onKeepPlanning?: () => void
}) {
  const { decide } = useRequestActions(request)
  return (
    <div className={PANEL} role='alertdialog' aria-label={m.request_waiting()}>
      <PanelTitle icon={TaskDone01Icon}>{m.request_plan_title()}</PanelTitle>
      <div className='bg-muted/40 max-h-72 overflow-auto rounded-md px-3 py-2'>
        <Markdown text={plan} className='text-sm' />
      </div>
      <div className='flex justify-end gap-1.5'>
        <Button
          size='sm'
          variant='ghost'
          onClick={() => {
            if (onKeepPlanning) onKeepPlanning()
            else void decide('decline')
          }}
        >
          {m.request_plan_keep_planning()}
        </Button>
        <Button size='sm' onClick={() => void decide('accept')}>
          {m.request_plan_implement()}
        </Button>
      </div>
    </div>
  )
}

function QuestionsPanel({
  request,
  content
}: {
  request: RequestItem
  content: QuestionsContent
}) {
  const { progress, pick, back, advance } = useRequestActions(request)
  const { questions } = content
  const question = questions[progress.index]
  const total = questions.length
  const last = progress.index === total - 1

  useEffect(() => {
    if (!question) return
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.altKey || event.ctrlKey || event.metaKey) return
      const target = event.target
      if (
        target instanceof HTMLElement &&
        (target.isContentEditable ||
          target.tagName === 'INPUT' ||
          target.tagName === 'TEXTAREA')
      )
        return
      const option = question.options[Number(event.key) - 1]
      if (!option) return
      event.preventDefault()
      pick(option.label, question.multiSelect)
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [question, pick])

  if (!question) return null
  const preview = question.options.find(
    (option) => option.preview && progress.selected.includes(option.label)
  )?.preview

  return (
    <div className={PANEL} role='alertdialog' aria-label={m.request_waiting()}>
      <PanelTitle
        icon={UserQuestion01Icon}
        aside={
          total > 1 ? (
            <span className='text-muted-foreground shrink-0 text-xs'>
              {m.request_question_count({
                current: progress.index + 1,
                total
              })}
            </span>
          ) : null
        }
      >
        <span className='text-muted-foreground me-2 text-xs font-normal'>
          {question.header}
        </span>
        {question.question}
      </PanelTitle>
      {question.multiSelect ? (
        <p className='text-muted-foreground text-xs'>
          {m.request_question_multi()}
        </p>
      ) : null}
      <ol className='flex flex-col gap-1'>
        {question.options.map((option, index) => {
          const selected = progress.selected.includes(option.label)
          return (
            <li key={option.label}>
              <button
                type='button'
                aria-pressed={selected}
                onClick={() => pick(option.label, question.multiSelect)}
                className={cn(
                  'hover:bg-muted focus-visible:ring-ring flex w-full items-start gap-2.5 rounded-lg px-2 py-1.5 text-left text-sm outline-none focus-visible:ring-2',
                  selected && 'bg-accent'
                )}
              >
                {index < 9 ? <Kbd className='mt-0.5'>{index + 1}</Kbd> : null}
                <span className='flex min-w-0 flex-col'>
                  <span className='text-foreground'>{option.label}</span>
                  {option.description && option.description !== option.label ? (
                    <span className='text-muted-foreground text-xs'>
                      {option.description}
                    </span>
                  ) : null}
                </span>
              </button>
            </li>
          )
        })}
      </ol>
      {preview ? (
        <div className='bg-muted/40 max-h-48 overflow-auto rounded-md px-3 py-2'>
          <Markdown text={preview} className='text-xs' />
        </div>
      ) : null}
      {progress.index > 0 || question.multiSelect ? (
        <div className='flex justify-end gap-1.5'>
          {progress.index > 0 ? (
            <Button size='sm' variant='ghost' onClick={back}>
              {m.request_question_back()}
            </Button>
          ) : null}
          {question.multiSelect ? (
            <Button
              size='sm'
              disabled={progress.selected.length === 0}
              onClick={() => advance()}
            >
              {last ? m.request_question_submit() : m.request_question_next()}
            </Button>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}
