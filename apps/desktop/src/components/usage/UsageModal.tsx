import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { type ReactNode, useEffect, useState } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import { useUsageStore } from '@/state/usage-store'
import { useModelsStore } from '@/state/models-store'
import { useNavigate } from '@tanstack/react-router'
import type { UsageWindow } from '@/bindings/UsageWindow'
import type { BankedResets } from '@/bindings/BankedResets'
import { Button } from '@hoshi/ui/components/button'
import { Skeleton } from '@hoshi/ui/components/skeleton'
import { metaOf } from '@/components/brand/provider-meta'
import { IconButton } from '@hoshi/ui/components/icon-button'
import { PageModal, PageModalClose } from '@hoshi/ui/components/page-modal'
import type { BankedResetSpend } from '@/bindings/BankedResetSpend'
import type { BankedResetFailure } from '@/bindings/BankedResetFailure'
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
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@hoshi/ui/components/tooltip'
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
  ArrowDownRight01Icon,
  ArrowRight01Icon,
  ArrowUpRight01Icon,
  InfinityIcon,
  RefreshIcon,
  Ticket01Icon
} from '@hugeicons/core-free-icons'
import {
  type Duration,
  type Pace,
  paceOf,
  remainingPercent,
  spendableResets,
  timeLeftPercent,
  timeToExpiry,
  timeToReset
} from '@/lib/usage'

const MINUTE = 60_000
const LOW_PERCENT = 10

const PACE: Record<Pace, { icon: typeof RefreshIcon; label: () => string }> = {
  ahead: { icon: ArrowUpRight01Icon, label: m.usage_pace_ahead },
  on: { icon: ArrowRight01Icon, label: m.usage_pace_on },
  under: { icon: ArrowDownRight01Icon, label: m.usage_pace_under }
}

export function UsageModal() {
  const navigate = useNavigate()
  const limits = useUsageStore((state) => state.limits)
  const checking = useUsageStore((state) => state.checking)
  const failed = useUsageStore((state) => state.failed)
  const refresh = useUsageStore((state) => state.refresh)
  const provider = useModelsStore((state) => state.catalog?.provider)
  const meta = metaOf(provider)
  const now = useNow()

  useEffect(() => {
    void refresh()
  }, [refresh])

  const banked = spendableResets(limits)

  let content: ReactNode
  if (limits && limits.windows.length > 0) {
    content = limits.windows.map((window) => {
      const resets =
        window.kind === 'session' && window.model === null ? banked : null
      return (
        <WindowCard
          key={`${window.kind}:${window.model ?? ''}`}
          window={window}
          now={now}
          bar={meta.bar}
          banked={resets}
          footer={
            resets ? (
              <BankedResetsFooter
                banked={resets}
                now={now}
                checking={checking}
              />
            ) : null
          }
        />
      )
    })
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
    content = <WindowCardsSkeleton />
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
        <section className='mx-auto flex max-w-3xl flex-col gap-3 px-6 py-8'>
          <h2 className='flex items-center gap-2 text-sm font-medium'>
            <meta.icon className={cn('size-3.5', meta.tint)} />
            {meta.label()}
          </h2>
          {content}
        </section>
      </div>
    </PageModal>
  )
}

function WindowCard({
  window,
  now,
  bar,
  banked,
  footer
}: {
  window: UsageWindow
  now: number
  bar: string
  banked: BankedResets | null
  footer: ReactNode
}) {
  const left = remainingPercent(window)
  const mark = timeLeftPercent(window, now)
  const pace = paceOf(window, now)
  const reset = timeToReset(window, now)
  const meter = (
    <>
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
    </>
  )

  return (
    <div className='border-border rounded-xl border'>
      <div className='flex items-center gap-6 px-4 py-3'>
        <div className='flex w-40 shrink-0 flex-col gap-0.5'>
          <span className='truncate text-sm font-medium'>
            {windowLabel(window)}
          </span>
          <span className='flex items-baseline gap-1.5'>
            <span className='text-2xl font-semibold tabular-nums'>{left}%</span>
            <span className='text-muted-foreground text-xs'>
              {m.usage_left()}
            </span>
            {pace ? (
              <HugeiconsIcon
                icon={PACE[pace].icon}
                strokeWidth={2}
                aria-label={PACE[pace].label()}
                className='text-muted-foreground size-3.5 self-center'
              />
            ) : null}
          </span>
        </div>
        {pace === null && mark === null ? (
          <div className='relative h-7 min-w-0 flex-1'>{meter}</div>
        ) : (
          <Tooltip>
            <TooltipTrigger
              render={
                <div className='relative h-7 min-w-0 flex-1 cursor-default' />
              }
            >
              {meter}
            </TooltipTrigger>
            <TooltipContent side='top' className='flex flex-col gap-0.5'>
              {pace ? <span>{PACE[pace].label()}</span> : null}
              {mark === null ? null : <span>{m.usage_pace_mark()}</span>}
            </TooltipContent>
          </Tooltip>
        )}
        <span className='text-muted-foreground flex w-36 shrink-0 items-center justify-end gap-1.5 text-xs tabular-nums'>
          {reset ? m.usage_resets_in({ time: formatDuration(reset) }) : null}
          {banked ? (
            <>
              {reset ? <span aria-hidden>·</span> : null}
              <span
                role='img'
                aria-label={m.usage_banked_resets_label({
                  count: banked.count
                })}
                className='flex items-center gap-1'
              >
                <HugeiconsIcon
                  icon={Ticket01Icon}
                  strokeWidth={2}
                  className='size-3.5'
                />
                {banked.count}
              </span>
            </>
          ) : null}
        </span>
      </div>
      {footer}
    </div>
  )
}

function BankedResetsFooter({
  banked,
  now,
  checking
}: {
  banked: BankedResets
  now: number
  checking: boolean
}) {
  const spending = useUsageStore((state) => state.spending)
  const spendBankedReset = useUsageStore((state) => state.spendBankedReset)
  const [confirming, setConfirming] = useState(false)
  const [result, setResult] = useState<BankedResetSpend | null>(null)
  const expiry = timeToExpiry(banked, now)
  const busy = checking || spending

  const spend = async () => {
    setConfirming(false)
    setResult(null)
    setResult(await spendBankedReset())
  }

  return (
    <div className='border-border text-muted-foreground flex flex-wrap items-center gap-x-3 gap-y-1 border-t px-4 py-2 text-xs'>
      <span className='tabular-nums'>
        {expiry
          ? m.usage_banked_resets_expiring({
              count: banked.count,
              time: formatDuration(expiry)
            })
          : m.usage_banked_resets({ count: banked.count })}
      </span>
      {result ? (
        <span
          role='status'
          className={cn(
            result.status === 'failed' ? 'text-status-error' : 'text-foreground'
          )}
        >
          {spendMessage(result)}
        </span>
      ) : null}
      <Button
        variant='outline'
        size='xs'
        className='ms-auto'
        disabled={busy}
        onClick={() => setConfirming(true)}
      >
        {spending ? m.usage_using_reset() : m.usage_use_reset()}
      </Button>
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

function WindowCardsSkeleton() {
  return (
    <div role='status' aria-busy className='flex flex-col gap-3'>
      <span className='sr-only'>{m.usage_checking()}</span>
      {[0, 1].map((index) => (
        <div
          key={index}
          className='border-border flex items-center gap-6 rounded-xl border px-4 py-3'
        >
          <div className='flex w-40 shrink-0 flex-col gap-0.5'>
            <span className='flex h-5 items-center'>
              <Skeleton className='h-3 w-16' />
            </span>
            <span className='flex h-8 items-center'>
              <Skeleton className='h-6 w-24' />
            </span>
          </div>
          <Skeleton className='h-4 min-w-0 flex-1 rounded-full' />
          <span className='flex w-36 shrink-0 justify-end'>
            <Skeleton className='h-3 w-20' />
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

function formatDuration({ days, hours, minutes }: Duration) {
  if (days > 0) return m.duration_days_hours({ days, hours })
  if (hours > 0) return m.duration_hours_minutes({ hours, minutes })
  return m.duration_minutes({ minutes })
}

function useNow() {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), MINUTE)
    return () => clearInterval(timer)
  }, [])
  return now
}
