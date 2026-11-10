import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { type ReactNode, useEffect, useState } from 'react'
import { getLocale } from '@/paraglide/runtime'
import { HugeiconsIcon } from '@hugeicons/react'
import { useUsageStore } from '@/state/usage-store'
import { useModelsStore } from '@/state/models-store'
import { useNavigate } from '@tanstack/react-router'
import type { UsageWindow } from '@/bindings/UsageWindow'
import type { ProviderPlan } from '@/bindings/ProviderPlan'
import type { BankedResets } from '@/bindings/BankedResets'
import { Button } from '@hoshi/ui/components/button'
import { Skeleton } from '@hoshi/ui/components/skeleton'
import { metaOf } from '@/components/brand/provider-meta'
import { IconButton } from '@hoshi/ui/components/icon-button'
import type { ProviderAccount } from '@/bindings/ProviderAccount'
import { PageModal, PageModalClose } from '@hoshi/ui/components/page-modal'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@hoshi/ui/components/tooltip'
import type { BankedResetSpend } from '@/bindings/BankedResetSpend'
import type { BankedResetFailure } from '@/bindings/BankedResetFailure'
import {
  HoverCard,
  HoverCardContent,
  HoverCardTrigger
} from '@hoshi/ui/components/hover-card'
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle
} from '@hoshi/ui/components/alert-dialog'
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle
} from '@hoshi/ui/components/empty'
import {
  Alert02Icon,
  ArrowRight01Icon,
  InfinityIcon,
  RefreshIcon,
  Ticket01Icon
} from '@hugeicons/core-free-icons'
import {
  bankedResetsFor,
  type Duration,
  evenPaceLeft,
  remainingPercent,
  resetMoment,
  restoredPercent,
  spendableResets,
  timeLeftPercent,
  timeToExpiry,
  timeToReset
} from '@/lib/usage'

const MINUTE = 60_000
const LOW_PERCENT = 10
const HOVER_DELAY_MS = 300

const PLAN: Record<ProviderPlan, () => string> = {
  pro: m.provider_plan_pro,
  max: m.provider_plan_max,
  max5x: m.provider_plan_max5x,
  max20x: m.provider_plan_max20x,
  team: m.provider_plan_team,
  enterprise: m.provider_plan_enterprise
}

type BankedSpend = {
  banked: BankedResets
  busy: boolean
  spending: boolean
  confirming: boolean
  confirm: () => void
}

export function UsageModal() {
  const navigate = useNavigate()
  const limits = useUsageStore((state) => state.limits)
  const checking = useUsageStore((state) => state.checking)
  const failed = useUsageStore((state) => state.failed)
  const refresh = useUsageStore((state) => state.refresh)
  const spending = useUsageStore((state) => state.spending)
  const spendBankedReset = useUsageStore((state) => state.spendBankedReset)
  const provider = useModelsStore((state) => state.catalog?.provider)
  const meta = metaOf(provider)
  const now = useNow()
  const [confirming, setConfirming] = useState(false)
  const [result, setResult] = useState<BankedResetSpend | null>(null)

  useEffect(() => {
    void refresh()
  }, [refresh])

  const banked = spendableResets(limits)
  const busy = checking || spending

  const spend = async () => {
    setConfirming(false)
    setResult(null)
    setResult(await spendBankedReset())
  }

  let content: ReactNode
  if (limits && limits.windows.length > 0) {
    const bankedSpend: BankedSpend | null = banked
      ? {
          banked,
          busy,
          spending,
          confirming,
          confirm: () => setConfirming(true)
        }
      : null
    content = (
      <ul className='border-border divide-border divide-y overflow-hidden rounded-xl border'>
        {limits.windows.map((window) => (
          <WindowRow
            key={`${window.kind}:${window.model ?? ''}`}
            window={window}
            now={now}
            bar={meta.bar}
            spend={bankedResetsFor(window, limits) ? bankedSpend : null}
          />
        ))}
      </ul>
    )
  } else if (limits) {
    content = (
      <UsageEmpty
        icon={InfinityIcon}
        title={m.usage_no_limits()}
        description={m.usage_no_limits_description()}
      />
    )
  } else if (failed && !checking) {
    content = (
      <UsageEmpty
        icon={Alert02Icon}
        title={m.usage_unavailable()}
        description={m.usage_unavailable_description()}
        action={
          <Button
            variant='outline'
            size='sm'
            onClick={() => void refresh({ force: true })}
          >
            {m.usage_try_again()}
          </Button>
        }
      />
    )
  } else {
    content = <WindowRowsSkeleton />
  }

  return (
    <PageModal
      label={m.usage()}
      topBar={<div data-tauri-drag-region className='h-10' />}
      onClose={() => navigate({ to: '/' })}
    >
      <header className='border-border flex h-12 shrink-0 items-center gap-3 border-b ps-5 pe-2'>
        <nav
          aria-label={m.usage()}
          className='flex min-w-0 items-center gap-1.5 text-sm'
        >
          <span className='text-muted-foreground'>{m.usage()}</span>
          <HugeiconsIcon
            icon={ArrowRight01Icon}
            strokeWidth={2}
            className='text-muted-foreground size-3.5'
          />
          <span aria-current='page' className='truncate font-medium'>
            {m.usage_limits()}
          </span>
        </nav>
        <div className='ms-auto flex items-center gap-0.5'>
          <IconButton
            label={m.usage_refresh()}
            disabled={checking}
            onClick={() => void refresh({ force: true })}
          >
            <HugeiconsIcon
              icon={RefreshIcon}
              strokeWidth={2}
              className={cn(checking && 'animate-spin')}
            />
          </IconButton>
          <PageModalClose label={m.close_usage()} />
        </div>
      </header>
      <div className='min-h-0 flex-1 overflow-y-auto'>
        <section className='mx-auto flex max-w-3xl flex-col gap-4 px-6 py-8'>
          <AccountHeader meta={meta} account={limits?.account ?? null} />
          {content}
          {result ? (
            <p
              role='status'
              className={cn(
                'text-xs',
                result.status === 'failed'
                  ? 'text-status-error'
                  : 'text-muted-foreground'
              )}
            >
              {spendMessage(result)}
            </p>
          ) : null}
        </section>
      </div>
      {banked ? (
        <AlertDialog open={confirming} onOpenChange={setConfirming}>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>{m.usage_use_reset_title()}</AlertDialogTitle>
              <AlertDialogDescription>
                {m.usage_use_reset_description({ count: banked.count - 1 })}
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>{m.cancel()}</AlertDialogCancel>
              <Button disabled={busy} onClick={() => void spend()}>
                {m.usage_use_reset()}
              </Button>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      ) : null}
    </PageModal>
  )
}

function AccountHeader({
  meta,
  account
}: {
  meta: ReturnType<typeof metaOf>
  account: ProviderAccount | null
}) {
  const plan = account?.plan ? PLAN[account.plan] : null
  return (
    <div className='flex items-center gap-3'>
      <span
        className={cn(
          'flex size-9 shrink-0 items-center justify-center rounded-lg',
          meta.tile
        )}
      >
        <meta.icon className='size-4.5' />
      </span>
      <div className='flex min-w-0 flex-col'>
        <h2 className='text-sm font-medium'>{meta.label()}</h2>
        {account?.email ? <AccountEmail email={account.email} /> : null}
      </div>
      {plan ? (
        <span className='border-border text-muted-foreground ms-auto shrink-0 rounded-full border px-2.5 py-0.5 text-xs font-medium'>
          {plan()}
        </span>
      ) : null}
    </div>
  )
}

/** Blurred until clicked, so the email stays private while sharing the screen. */
function AccountEmail({ email }: { email: string }) {
  const [revealed, setRevealed] = useState(false)
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <button
            type='button'
            aria-label={revealed ? undefined : m.usage_reveal_email()}
            onClick={() => setRevealed(!revealed)}
            className='text-muted-foreground focus-visible:ring-ring min-w-0 cursor-pointer truncate rounded-sm text-start text-xs outline-none focus-visible:ring-2'
          />
        }
      >
        <span
          className={cn(
            'transition-[filter]',
            !revealed && 'blur-xs select-none'
          )}
        >
          {email}
        </span>
      </TooltipTrigger>
      <TooltipContent side='bottom' sideOffset={15}>
        {revealed ? m.usage_hide_email() : m.usage_reveal_email()}
      </TooltipContent>
    </Tooltip>
  )
}

function WindowRow({
  window,
  now,
  bar,
  spend
}: {
  window: UsageWindow
  now: number
  bar: string
  spend: BankedSpend | null
}) {
  const [hovered, setHovered] = useState(false)
  const left = remainingPercent(window)
  const mark = timeLeftPercent(window, now)
  const reset = timeToReset(window, now)
  const pinned = Boolean(spend && (spend.confirming || spend.spending))

  return (
    <HoverCard open={hovered || pinned} onOpenChange={setHovered}>
      <HoverCardTrigger
        delay={HOVER_DELAY_MS}
        closeDelay={100}
        render={
          <li
            // biome-ignore lint/a11y/noNoninteractiveTabindex: focusing a row opens its detail
            tabIndex={0}
            className='hover:bg-muted/50 focus-visible:bg-muted/50 flex cursor-default items-center gap-4 px-4 py-3 outline-none'
          />
        }
      >
        <span className='w-36 shrink-0 truncate text-sm'>
          {windowLabel(window)}
        </span>
        <div className='relative h-5 min-w-0 flex-1'>
          <div className='bg-muted absolute inset-x-0 inset-y-1.5 rounded-full' />
          <div
            className={cn(
              'absolute inset-y-1.5 left-0 rounded-full transition-[width] duration-500',
              left <= LOW_PERCENT ? 'bg-status-error/60' : bar
            )}
            style={{ width: `${left}%` }}
          />
          {mark === null ? null : (
            <div
              className='bg-foreground absolute inset-y-0.5 w-0.5 -translate-x-1/2 rounded-full'
              style={{ left: `${mark}%` }}
            />
          )}
        </div>
        <span className='flex w-20 shrink-0 items-baseline justify-end gap-1'>
          <span className='text-sm font-semibold tabular-nums'>{left}%</span>
          <span className='text-muted-foreground text-xs'>
            {m.usage_left()}
          </span>
        </span>
        <span className='text-muted-foreground flex w-28 shrink-0 items-center justify-end gap-1.5 text-xs tabular-nums'>
          {reset ? formatDuration(reset) : null}
          {spend ? (
            <>
              {reset ? <span aria-hidden>·</span> : null}
              <span
                role='img'
                aria-label={m.usage_banked_resets_label({
                  count: spend.banked.count
                })}
                className='flex items-center gap-1'
              >
                <HugeiconsIcon
                  icon={Ticket01Icon}
                  strokeWidth={2}
                  className='size-3.5'
                />
                {spend.banked.count}
              </span>
            </>
          ) : null}
        </span>
      </HoverCardTrigger>
      <HoverCardContent
        side='bottom'
        align='end'
        sideOffset={6}
        className='w-80 p-0'
      >
        <WindowDetail window={window} now={now} />
        {spend ? <BankedResetsFooter spend={spend} now={now} /> : null}
      </HoverCardContent>
    </HoverCard>
  )
}

function WindowDetail({ window, now }: { window: UsageWindow; now: number }) {
  const reset = timeToReset(window, now)
  const moment = resetMoment(window, now, getLocale())
  const pace = evenPaceLeft(window, now)
  const rows: [string, string][] = [
    [m.usage_detail_left(), `${remainingPercent(window)}%`]
  ]
  if (moment && reset) {
    rows.push([
      m.usage_detail_resets(),
      m.usage_reset_when({
        when: resetWhen(moment),
        time: formatDuration(reset)
      })
    ])
  }
  if (pace !== null) {
    rows.push([
      m.usage_detail_even_pace(),
      m.usage_even_pace_left({ percent: pace })
    ])
  }
  rows.push([
    m.usage_detail_restores(),
    m.usage_restores_at_reset({ percent: restoredPercent(window) })
  ])

  return (
    <dl className='grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 p-3 text-xs'>
      {rows.map(([term, value]) => (
        <div key={term} className='contents'>
          <dt className='text-muted-foreground'>{term}</dt>
          <dd className='tabular-nums'>{value}</dd>
        </div>
      ))}
    </dl>
  )
}

function BankedResetsFooter({
  spend,
  now
}: {
  spend: BankedSpend
  now: number
}) {
  const { banked } = spend
  const expiry = timeToExpiry(banked, now)

  return (
    <div className='border-border text-muted-foreground flex items-center gap-2 border-t px-3 py-2 text-xs'>
      <HugeiconsIcon icon={Ticket01Icon} strokeWidth={2} className='size-3.5' />
      <span className='tabular-nums'>
        {expiry
          ? m.usage_banked_resets_expiring({
              count: banked.count,
              time: formatExpiry(expiry)
            })
          : m.usage_banked_resets({ count: banked.count })}
      </span>
      <Button
        variant='outline'
        size='xs'
        className='ms-auto'
        disabled={spend.busy}
        onClick={spend.confirm}
      >
        {spend.spending ? m.usage_using_reset() : m.usage_use_reset()}
      </Button>
    </div>
  )
}

function spendMessage(result: BankedResetSpend) {
  switch (result.status) {
    case 'reset':
      return m.usage_reset_done()
    case 'nothingToReset':
      return m.usage_reset_nothing_to_reset()
    case 'alreadyUsed':
      return m.usage_reset_already_used()
    case 'noneLeft':
      return m.usage_reset_none_left()
    case 'failed':
      return failureMessage(result.reason)
  }
}

function failureMessage(reason: BankedResetFailure) {
  switch (reason) {
    case 'loginUnreadable':
      return m.usage_reset_login_unreadable()
    case 'signedOut':
      return m.usage_reset_signed_out()
    case 'rateLimited':
      return m.usage_reset_rate_limited()
    case 'coolingDown':
      return m.usage_reset_cooling_down()
    case 'unconfirmed':
      return m.usage_reset_unconfirmed()
    case 'requestFailed':
      return m.usage_reset_request_failed()
    case 'limitsUnread':
      return m.usage_reset_limits_unread()
  }
}

function WindowRowsSkeleton() {
  return (
    <div
      role='status'
      aria-busy
      className='border-border divide-border divide-y rounded-xl border'
    >
      <span className='sr-only'>{m.usage_checking()}</span>
      {[0, 1, 2].map((index) => (
        <div key={index} className='flex items-center gap-4 px-4 py-3'>
          <span className='flex h-5 w-36 shrink-0 items-center'>
            <Skeleton className='h-3 w-16' />
          </span>
          <Skeleton className='h-2 min-w-0 flex-1 rounded-full' />
          <span className='flex w-20 shrink-0 justify-end'>
            <Skeleton className='h-3 w-12' />
          </span>
          <span className='flex w-28 shrink-0 justify-end'>
            <Skeleton className='h-3 w-14' />
          </span>
        </div>
      ))}
    </div>
  )
}

function UsageEmpty({
  icon,
  title,
  description,
  action
}: {
  icon: typeof RefreshIcon
  title: string
  description: string
  action?: ReactNode
}) {
  return (
    <Empty className='border-border border py-10'>
      <EmptyHeader>
        <EmptyMedia variant='icon'>
          <HugeiconsIcon icon={icon} strokeWidth={2} />
        </EmptyMedia>
        <EmptyTitle>{title}</EmptyTitle>
        <EmptyDescription>{description}</EmptyDescription>
      </EmptyHeader>
      {action ? <EmptyContent>{action}</EmptyContent> : null}
    </Empty>
  )
}

function windowLabel(window: UsageWindow) {
  if (window.kind === 'session') return m.usage_session()
  return window.model
    ? m.usage_weekly_model({ model: window.model })
    : m.usage_weekly()
}

function resetWhen({ day, text }: NonNullable<ReturnType<typeof resetMoment>>) {
  if (day === 'today') return m.usage_reset_today({ time: text })
  if (day === 'tomorrow') return m.usage_reset_tomorrow({ time: text })
  return text
}

function formatDuration({ days, hours, minutes }: Duration) {
  if (days > 0) return m.duration_days_hours({ days, hours })
  if (hours > 0) return m.duration_hours_minutes({ hours, minutes })
  return m.duration_minutes({ minutes })
}

function formatExpiry(duration: Duration) {
  return duration.days > 0
    ? m.duration_days({ days: duration.days })
    : formatDuration(duration)
}

function useNow() {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), MINUTE)
    return () => clearInterval(timer)
  }, [])
  return now
}
