import { cn } from 'cn'
import { Toast } from '@base-ui/react/toast'
import { HugeiconsIcon } from '@hugeicons/react'
import { CopyButton } from '@hoshi/ui/components/copy-button'
import { Button, buttonVariants } from '@hoshi/ui/components/button'
import {
  type ComponentPropsWithoutRef,
  createContext,
  type CSSProperties,
  type KeyboardEvent,
  type ReactNode,
  useContext,
  useEffect,
  useState
} from 'react'
import {
  buildVisibleToastLayout,
  hasVisibleToastAction,
  shouldHideCollapsedToastContent,
  shouldRenderScopedToast
} from '@hoshi/ui/lib/toast-layout'
import {
  Alert02Icon,
  AlertCircleIcon,
  ArrowDown01Icon,
  ArrowUp01Icon,
  Cancel01Icon,
  CheckmarkCircle02Icon,
  Copy01Icon,
  InformationCircleIcon,
  Loading03Icon
} from '@hugeicons/core-free-icons'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@hoshi/ui/components/tooltip'

type ButtonVariant = NonNullable<
  Parameters<typeof buttonVariants>[0]
>['variant']

export type ToastData = {
  /** Shown only while this scope, such as a thread, is the active one. */
  scope?: string | null
  leadingIcon?: ReactNode
  /** A small label in the style of a tooltip; for anchored toasts. */
  tooltipStyle?: boolean
  onClose?: (() => void) | undefined
  /** Closes after this long on screen, counting only while the window is seen. */
  dismissAfterVisibleMs?: number
  /** An error's description is copyable unless this is set. */
  hideCopyButton?: boolean
  additionalActions?: ReadonlyArray<{
    id: string
    props: ComponentPropsWithoutRef<'button'>
  }>
  secondaryActionProps?: ComponentPropsWithoutRef<'button'>
  secondaryActionVariant?: ButtonVariant
  expandableContent?: ReactNode
  expandableLabels?: { expand?: string; collapse?: string }
  /** The description itself opens the expandable content. */
  expandableDescriptionTrigger?: boolean
  actionLayout?: 'inline' | 'stacked-end'
  actionVariant?: ButtonVariant
}

/** What the toasts say themselves; the app passes them, translated. */
export type ToastLabels = {
  dismiss: string
  copyError: string
  copiedError: string
  showDetails: string
  hideDetails: string
}

type BaseToastManager = ReturnType<typeof Toast.createToastManager<ToastData>>
type ToastManagerListener = Parameters<BaseToastManager[' subscribe']>[0]
type ToastManagerEvent = Parameters<ToastManagerListener>[0]

/**
 * Base UI's manager drops what it gets while no provider listens. Toasts raised before the
 * first render wait until one does.
 */
function bufferedToastManager(): BaseToastManager {
  const manager = Toast.createToastManager<ToastData>()
  const listeners = new Set<ToastManagerListener>()
  const pending: ToastManagerEvent[] = []
  manager[' subscribe']((event) => {
    if (listeners.size === 0) {
      pending.push(event)
      return
    }
    for (const listener of listeners) listener(event)
  })
  return {
    ...manager,
    ' subscribe'(listener) {
      listeners.add(listener)
      for (const event of pending.splice(0)) listener(event)
      return () => {
        listeners.delete(listener)
      }
    }
  }
}

/** Raises toasts from anywhere, React or not: `toastManager.add({ type, title, ... })`. */
const toastManager = bufferedToastManager()
/** Raises toasts beside an element, given as `positionerProps.anchor`. */
const anchoredToastManager = bufferedToastManager()
type ToastId = ReturnType<typeof toastManager.add>
const visibleTimeRemainingMs = new Map<ToastId, number>()

const TOAST_ICONS = {
  error: AlertCircleIcon,
  info: InformationCircleIcon,
  loading: Loading03Icon,
  success: CheckmarkCircle02Icon,
  warning: Alert02Icon
} as const

const ERROR_DESCRIPTION_CLAMP_MIN_CHARS = 180

function errorDescriptionClampClass(type: unknown, description: unknown) {
  if (type !== 'error' || typeof description !== 'string') return undefined
  if (description.length < ERROR_DESCRIPTION_CLAMP_MIN_CHARS) return undefined
  return 'line-clamp-4'
}

const LabelsContext = createContext<ToastLabels | null>(null)

function useLabels(): ToastLabels {
  const labels = useContext(LabelsContext)
  if (!labels) throw new Error('toasts need a provider with their labels')
  return labels
}

const CORNER_DISMISS = 'absolute -top-1.5 -right-1.5 z-20'
const CORNER_ORB = cn(
  'border-border/60 bg-popover/92 text-muted-foreground inline-flex size-6 shrink-0 cursor-pointer items-center justify-center rounded-full border shadow-sm backdrop-blur-sm outline-none',
  'hover:bg-popover hover:text-foreground transition-[color,background-color,box-shadow]',
  'focus-visible:ring-ring focus-visible:ring-offset-background focus-visible:ring-2 focus-visible:ring-offset-1 focus-visible:ring-inset'
)

function DismissButton({
  manager,
  toastId,
  onClose
}: {
  manager: BaseToastManager
  toastId: ToastId
  onClose: (() => void) | undefined
}) {
  const labels = useLabels()
  return (
    <div className={CORNER_DISMISS}>
      <button
        type='button'
        aria-label={labels.dismiss}
        data-slot='toast-close'
        className={CORNER_ORB}
        onClick={() => {
          onClose?.()
          manager.close(toastId)
        }}
      >
        <HugeiconsIcon
          icon={Cancel01Icon}
          strokeWidth={2.25}
          className='size-3'
        />
      </button>
    </div>
  )
}

const EXPANDABLE_PANEL =
  'mt-2 max-h-40 min-h-0 overflow-y-auto overscroll-contain pr-0.5 select-text'

function ToastExpandableSection({
  children,
  labels
}: {
  children: ReactNode
  labels: { expand: string; collapse: string }
}) {
  const [open, setOpen] = useState(false)
  return (
    <div className='min-w-0'>
      <button
        type='button'
        aria-expanded={open}
        className='text-muted-foreground hover:text-foreground inline-flex cursor-pointer items-center gap-1 rounded-md py-0.5 text-left text-xs font-medium transition-colors'
        onClick={() => setOpen((previous) => !previous)}
      >
        <HugeiconsIcon
          icon={open ? ArrowUp01Icon : ArrowDown01Icon}
          strokeWidth={2.25}
          className='size-3.5 shrink-0 opacity-80'
        />
        {open ? labels.collapse : labels.expand}
      </button>
      {open ? <div className={EXPANDABLE_PANEL}>{children}</div> : null}
    </div>
  )
}

function ToastDescriptionAndExpandable({
  toastData,
  toastDescription,
  toastType
}: {
  toastData: ToastData | undefined
  toastDescription: unknown
  toastType: unknown
}) {
  const defaults = useLabels()
  const [open, setOpen] = useState(false)
  const expandableContent = toastData?.expandableContent
  const labels = {
    expand: toastData?.expandableLabels?.expand ?? defaults.showDetails,
    collapse: toastData?.expandableLabels?.collapse ?? defaults.hideDetails
  }
  const descriptionClassName = cn(
    'text-muted-foreground min-w-0 wrap-break-word select-text',
    errorDescriptionClampClass(toastType, toastDescription)
  )

  if (!expandableContent) {
    return (
      <Toast.Description
        className={descriptionClassName}
        data-slot='toast-description'
      />
    )
  }

  if (!toastData?.expandableDescriptionTrigger) {
    return (
      <>
        <Toast.Description
          className={descriptionClassName}
          data-slot='toast-description'
        />
        <ToastExpandableSection labels={labels}>
          {expandableContent}
        </ToastExpandableSection>
      </>
    )
  }

  const toggle = () => setOpen((previous) => !previous)
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault()
      toggle()
    }
  }
  const label = open ? labels.collapse : labels.expand

  return (
    <>
      <Tooltip>
        <TooltipTrigger
          render={
            // biome-ignore lint/a11y/useSemanticElements: it holds the description's block content
            <div
              role='button'
              tabIndex={0}
              aria-label={label}
              aria-expanded={open}
              onClick={toggle}
              onKeyDown={onKeyDown}
              className={cn(
                'group ring-offset-background flex w-full min-w-0 cursor-pointer items-start gap-1.5 rounded-sm text-left outline-none select-none',
                'hover:bg-muted/40 transition-colors',
                'focus-visible:ring-ring focus-visible:ring-offset-background focus-visible:ring-2 focus-visible:ring-offset-1 focus-visible:ring-inset'
              )}
            />
          }
        >
          <div className='min-w-0 flex-1'>
            <Toast.Description
              className={cn(
                'text-muted-foreground min-w-0 wrap-break-word select-none',
                errorDescriptionClampClass(toastType, toastDescription),
                'decoration-muted-foreground/60 underline-offset-2 group-hover:underline'
              )}
              data-slot='toast-description'
            />
          </div>
          <HugeiconsIcon
            icon={open ? ArrowUp01Icon : ArrowDown01Icon}
            strokeWidth={2.25}
            aria-hidden
            className='text-muted-foreground size-3.5 h-lh shrink-0 opacity-80'
          />
        </TooltipTrigger>
        <TooltipContent side='top'>{label}</TooltipContent>
      </Tooltip>
      {open ? (
        <div className={EXPANDABLE_PANEL}>{expandableContent}</div>
      ) : null}
    </>
  )
}

type ToastBodyDescriptor = {
  icon: (typeof TOAST_ICONS)[keyof typeof TOAST_ICONS] | null
  stackedActionLayout: boolean
  actionVariant: ButtonVariant
  secondaryActionVariant: ButtonVariant
  copyErrorText: string | null
  hasTrailingControls: boolean
  inlineContentEndPad: string
}

function describeToastBody(toast: {
  type?: string | undefined
  description?: unknown
  actionProps?: unknown
  data?: ToastData | undefined
}): ToastBodyDescriptor {
  const icon = toast.type
    ? (TOAST_ICONS[toast.type as keyof typeof TOAST_ICONS] ?? null)
    : null
  const action = hasVisibleToastAction(toast.actionProps)
  const copyErrorText =
    toast.type === 'error' &&
    typeof toast.description === 'string' &&
    !toast.data?.hideCopyButton
      ? toast.description
      : null
  const hasTrailingControls =
    copyErrorText !== null ||
    action ||
    (toast.data?.additionalActions?.length ?? 0) > 0 ||
    toast.data?.secondaryActionProps !== undefined
  return {
    icon,
    stackedActionLayout: action && toast.data?.actionLayout === 'stacked-end',
    actionVariant: toast.data?.actionVariant ?? 'default',
    secondaryActionVariant: toast.data?.secondaryActionVariant ?? 'outline',
    copyErrorText,
    hasTrailingControls,
    inlineContentEndPad: hasTrailingControls ? 'pr-6' : 'pr-3.5'
  }
}

function ToastBodyContent({
  stackedActionLayout,
  icon,
  copyErrorText,
  actionProps,
  actionVariant,
  secondaryActionVariant,
  hasTrailingControls,
  toastData,
  toastDescription,
  toastType
}: ToastBodyDescriptor & {
  actionProps: { children?: ReactNode } | undefined
  toastData: ToastData | undefined
  toastDescription: unknown
  toastType: unknown
}) {
  const labels = useLabels()
  const additionalActions = toastData?.additionalActions ?? []
  const secondaryActionProps = toastData?.secondaryActionProps
  const leadingIcon = toastData?.leadingIcon

  return (
    <>
      <div
        className={cn(
          'flex min-h-0 min-w-0 flex-col gap-0.5',
          stackedActionLayout ? 'pr-5' : 'flex-1'
        )}
      >
        <div
          className={cn(
            'flex min-w-0 gap-2',
            !stackedActionLayout && !hasTrailingControls && 'pr-6'
          )}
        >
          {leadingIcon ? (
            <div
              className='flex h-lh w-4 shrink-0 items-center justify-center'
              data-slot='toast-icon'
            >
              {leadingIcon}
            </div>
          ) : icon ? (
            <div
              className='flex h-lh w-4 shrink-0 items-center'
              data-slot='toast-icon'
            >
              <HugeiconsIcon
                icon={icon}
                strokeWidth={2}
                className='in-data-[type=error]:text-destructive in-data-[type=info]:text-status-working in-data-[type=success]:text-status-done in-data-[type=warning]:text-warning size-4 in-data-[type=loading]:animate-spin in-data-[type=loading]:opacity-80'
              />
            </div>
          ) : null}
          <Toast.Title
            className='min-w-0 font-medium wrap-break-word'
            data-slot='toast-title'
          />
        </div>
        <ToastDescriptionAndExpandable
          toastData={toastData}
          toastDescription={toastDescription}
          toastType={toastType}
        />
      </div>
      {hasTrailingControls ? (
        <div
          className={cn(
            'flex items-center gap-1.5',
            stackedActionLayout ? 'w-full justify-end' : 'shrink-0'
          )}
        >
          {copyErrorText !== null ? (
            <CopyButton
              value={copyErrorText}
              label={labels.copyError}
              copiedLabel={labels.copiedError}
              icon={Copy01Icon}
              side='top'
              className='text-muted-foreground/80 hover:text-muted-foreground size-5 justify-center rounded-md'
              iconClassName='size-3'
            />
          ) : null}
          {additionalActions.map(({ id, props }) => (
            <Button
              {...props}
              key={id}
              type='button'
              size='xs'
              variant={secondaryActionVariant}
            />
          ))}
          {secondaryActionProps ? (
            <Button
              {...secondaryActionProps}
              type='button'
              size='xs'
              variant={secondaryActionVariant}
            />
          ) : null}
          {hasVisibleToastAction(actionProps) ? (
            <Toast.Action
              className={cn(
                buttonVariants({ size: 'xs', variant: actionVariant }),
                'shrink-0'
              )}
              data-slot='toast-action'
            >
              {actionProps?.children}
            </Toast.Action>
          ) : null}
        </div>
      ) : null}
    </>
  )
}

/** Closes a toast after it was on screen for `dismissAfterVisibleMs`, pausing while hidden. */
function VisibleAutoDismiss({
  toastId,
  dismissAfterVisibleMs
}: {
  toastId: ToastId
  dismissAfterVisibleMs: number | undefined
}) {
  useEffect(() => {
    if (!dismissAfterVisibleMs || dismissAfterVisibleMs <= 0) return
    let remainingMs =
      visibleTimeRemainingMs.get(toastId) ?? dismissAfterVisibleMs
    let startedAtMs: number | null = null
    let timeoutId: number | null = null
    let closed = false

    const clearTimer = () => {
      if (timeoutId === null) return
      window.clearTimeout(timeoutId)
      timeoutId = null
    }
    const close = () => {
      if (closed) return
      closed = true
      visibleTimeRemainingMs.delete(toastId)
      toastManager.close(toastId)
    }
    const pause = () => {
      if (startedAtMs === null) return
      remainingMs = Math.max(0, remainingMs - (Date.now() - startedAtMs))
      startedAtMs = null
      clearTimer()
      visibleTimeRemainingMs.set(toastId, remainingMs)
    }
    const start = () => {
      if (closed || startedAtMs !== null) return
      if (remainingMs <= 0) {
        close()
        return
      }
      startedAtMs = Date.now()
      clearTimer()
      timeoutId = window.setTimeout(() => {
        remainingMs = 0
        startedAtMs = null
        close()
      }, remainingMs)
    }
    const sync = () => {
      if (document.visibilityState === 'visible' && document.hasFocus()) start()
      else pause()
    }

    sync()
    document.addEventListener('visibilitychange', sync)
    window.addEventListener('focus', sync)
    window.addEventListener('blur', sync)
    return () => {
      document.removeEventListener('visibilitychange', sync)
      window.removeEventListener('focus', sync)
      window.removeEventListener('blur', sync)
      pause()
      clearTimer()
    }
  }, [dismissAfterVisibleMs, toastId])

  return null
}

export type ToastPosition =
  | 'top-left'
  | 'top-center'
  | 'top-right'
  | 'bottom-left'
  | 'bottom-center'
  | 'bottom-right'

type ToastProviderProps = Toast.Provider.Props & {
  labels: ToastLabels
  position?: ToastPosition
  /** The scope shown now; toasts of other scopes wait for theirs. */
  activeScope?: string | null
  /** On the viewport, such as the height of a top bar to keep clear as `--toast-header-offset`. */
  viewportClassName?: string
}

function ToastProvider({
  children,
  labels,
  position = 'top-right',
  activeScope = null,
  viewportClassName,
  ...props
}: ToastProviderProps) {
  return (
    <LabelsContext.Provider value={labels}>
      <Toast.Provider toastManager={toastManager} {...props}>
        {children}
        <Toasts
          position={position}
          activeScope={activeScope}
          viewportClassName={viewportClassName}
        />
      </Toast.Provider>
    </LabelsContext.Provider>
  )
}

const TOAST_SURFACE =
  'bg-popover text-popover-foreground ring-foreground/10 shadow-lg ring-1'

function Toasts({
  position,
  activeScope,
  viewportClassName
}: {
  position: ToastPosition
  activeScope: string | null
  viewportClassName: string | undefined
}) {
  const { toasts } = Toast.useToastManager<ToastData>()
  const isTop = position.startsWith('top')
  const visibleToasts = toasts.filter((toast) =>
    shouldRenderScopedToast(toast.data, activeScope)
  )
  const layout = buildVisibleToastLayout(visibleToasts)

  useEffect(() => {
    const active = new Set(toasts.map((toast) => toast.id))
    for (const toastId of visibleTimeRemainingMs.keys()) {
      if (!active.has(toastId)) visibleTimeRemainingMs.delete(toastId)
    }
  }, [toasts])

  return (
    <Toast.Portal data-slot='toast-portal'>
      <Toast.Viewport
        className={cn(
          'fixed z-100 mx-auto flex w-[calc(100%-var(--toast-inset)*2)] max-w-90 [--toast-inset:--spacing(4)] sm:[--toast-inset:--spacing(8)]',
          'data-[position*=top]:top-[calc(var(--toast-inset)+var(--toast-header-offset,0px))]',
          'data-[position*=bottom]:bottom-(--toast-inset)',
          'data-[position*=left]:left-(--toast-inset)',
          'data-[position*=right]:right-(--toast-inset)',
          'data-[position*=center]:left-1/2 data-[position*=center]:-translate-x-1/2',
          viewportClassName
        )}
        data-position={position}
        data-slot='toast-viewport'
        style={
          {
            '--toast-frontmost-height': `${layout.frontmostHeight}px`
          } as CSSProperties
        }
      >
        {layout.items.map(({ toast, visibleIndex, offsetY }) => {
          const hideCollapsedContent = shouldHideCollapsedToastContent(
            visibleIndex,
            layout.items.length
          )
          const body = describeToastBody(toast)

          return (
            <Toast.Root
              key={toast.id}
              toast={toast}
              data-position={position}
              swipeDirection={
                position.includes('center')
                  ? [isTop ? 'up' : 'down']
                  : position.includes('left')
                    ? ['left', isTop ? 'up' : 'down']
                    : ['right', isTop ? 'up' : 'down']
              }
              style={
                {
                  '--toast-index': visibleIndex,
                  '--toast-offset-y': `${offsetY}px`
                } as CSSProperties
              }
              className={cn(
                TOAST_SURFACE,
                'absolute z-[calc(9999-var(--toast-index))] w-full overflow-visible rounded-lg select-none [transition:transform_.5s_cubic-bezier(.22,1,.36,1),opacity_.5s,height_.15s]',
                'data-[position*=right]:right-0 data-[position*=right]:left-auto',
                'data-[position*=left]:right-auto data-[position*=left]:left-0',
                'data-[position*=center]:right-0 data-[position*=center]:left-0',
                'data-[position*=top]:top-0 data-[position*=top]:bottom-auto data-[position*=top]:origin-top',
                'data-[position*=bottom]:top-auto data-[position*=bottom]:bottom-0 data-[position*=bottom]:origin-bottom',
                'after:absolute after:left-0 after:h-[calc(var(--toast-gap)+1px)] after:w-full',
                'data-[position*=top]:after:top-full',
                'data-[position*=bottom]:after:bottom-full',
                visibleIndex > 0
                  ? 'not-data-expanded:[--toast-calc-height:var(--toast-frontmost-height)] data-expanded:[--toast-calc-height:max(var(--toast-frontmost-height,var(--toast-height)),var(--toast-height))]'
                  : '[--toast-calc-height:max(var(--toast-frontmost-height,var(--toast-height)),var(--toast-height))]',
                '[--toast-gap:--spacing(3)] [--toast-peek:--spacing(3)] [--toast-scale:calc(max(0,1-(var(--toast-index)*.1)))] [--toast-shrink:calc(1-var(--toast-scale))]',
                // Base UI measures with height:auto; --toast-height as min-height would keep it from shrinking.
                visibleIndex > 0
                  ? 'not-data-expanded:h-(--toast-calc-height) data-expanded:h-auto'
                  : 'h-auto',
                'data-[position*=top]:[--toast-calc-offset-y:calc(var(--toast-offset-y)+var(--toast-index)*var(--toast-gap)+var(--toast-swipe-movement-y))]',
                'data-[position*=bottom]:[--toast-calc-offset-y:calc(var(--toast-offset-y)*-1+var(--toast-index)*var(--toast-gap)*-1+var(--toast-swipe-movement-y))]',
                'data-[position*=top]:transform-[translateX(var(--toast-swipe-movement-x))_translateY(calc(var(--toast-swipe-movement-y)+(var(--toast-index)*var(--toast-peek))+(var(--toast-shrink)*var(--toast-calc-height))))_scale(var(--toast-scale))]',
                'data-[position*=bottom]:transform-[translateX(var(--toast-swipe-movement-x))_translateY(calc(var(--toast-swipe-movement-y)-(var(--toast-index)*var(--toast-peek))-(var(--toast-shrink)*var(--toast-calc-height))))_scale(var(--toast-scale))]',
                'data-limited:opacity-0',
                'data-position:data-expanded:transform-[translateX(var(--toast-swipe-movement-x))_translateY(var(--toast-calc-offset-y))]',
                'data-[position*=top]:data-starting-style:transform-[translateY(calc(-100%-var(--toast-inset)))]',
                'data-[position*=bottom]:data-starting-style:transform-[translateY(calc(100%+var(--toast-inset)))]',
                'data-[position*=top]:data-[position*=right]:data-starting-style:transform-[translateX(calc(100%+var(--toast-inset)))_translateY(var(--toast-calc-offset-y))]',
                'data-ending-style:opacity-0',
                'data-ending-style:not-data-limited:not-data-swipe-direction:transform-[translateY(calc(100%+var(--toast-inset)))]',
                'data-[position*=top]:data-[position*=right]:data-ending-style:not-data-limited:not-data-swipe-direction:transform-[translateX(calc(100%+var(--toast-inset)))_translateY(var(--toast-calc-offset-y))]',
                'data-ending-style:data-[swipe-direction=left]:transform-[translateX(calc(var(--toast-swipe-movement-x)-100%-var(--toast-inset)))_translateY(var(--toast-calc-offset-y))]',
                'data-ending-style:data-[swipe-direction=right]:transform-[translateX(calc(var(--toast-swipe-movement-x)+100%+var(--toast-inset)))_translateY(var(--toast-calc-offset-y))]',
                'data-ending-style:data-[swipe-direction=up]:transform-[translateY(calc(var(--toast-swipe-movement-y)-100%-var(--toast-inset)))]',
                'data-ending-style:data-[swipe-direction=down]:transform-[translateY(calc(var(--toast-swipe-movement-y)+100%+var(--toast-inset)))]',
                'data-expanded:data-ending-style:data-[swipe-direction=left]:transform-[translateX(calc(var(--toast-swipe-movement-x)-100%-var(--toast-inset)))_translateY(var(--toast-calc-offset-y))]',
                'data-expanded:data-ending-style:data-[swipe-direction=right]:transform-[translateX(calc(var(--toast-swipe-movement-x)+100%+var(--toast-inset)))_translateY(var(--toast-calc-offset-y))]',
                'data-expanded:data-ending-style:data-[swipe-direction=up]:transform-[translateY(calc(var(--toast-swipe-movement-y)-100%-var(--toast-inset)))]',
                'data-expanded:data-ending-style:data-[swipe-direction=down]:transform-[translateY(calc(var(--toast-swipe-movement-y)+100%+var(--toast-inset)))]'
              )}
            >
              <VisibleAutoDismiss
                toastId={toast.id}
                dismissAfterVisibleMs={toast.data?.dismissAfterVisibleMs}
              />
              <DismissButton
                manager={toastManager}
                toastId={toast.id}
                onClose={toast.data?.onClose}
              />
              <Toast.Content
                className={cn(
                  // overflow-x:clip keeps vertical overflow visible; hidden would clip detail panels.
                  'pointer-events-auto min-h-0 [overflow-x:clip] overflow-y-visible pl-3.5 text-sm transition-opacity duration-250 data-expanded:opacity-100',
                  body.stackedActionLayout
                    ? 'flex flex-col gap-2 py-2.5 pr-3.5'
                    : cn(
                        'flex items-center justify-between gap-1.5 py-3',
                        body.inlineContentEndPad
                      ),
                  hideCollapsedContent &&
                    'not-data-expanded:pointer-events-none not-data-expanded:opacity-0'
                )}
              >
                <ToastBodyContent
                  {...body}
                  actionProps={toast.actionProps}
                  toastData={toast.data}
                  toastDescription={toast.description}
                  toastType={toast.type}
                />
              </Toast.Content>
            </Toast.Root>
          )
        })}
      </Toast.Viewport>
    </Toast.Portal>
  )
}

type AnchoredToastProviderProps = Toast.Provider.Props & {
  labels: ToastLabels
  activeScope?: string | null
}

function AnchoredToastProvider({
  children,
  labels,
  activeScope = null,
  ...props
}: AnchoredToastProviderProps) {
  return (
    <LabelsContext.Provider value={labels}>
      <Toast.Provider toastManager={anchoredToastManager} {...props}>
        {children}
        <AnchoredToasts activeScope={activeScope} />
      </Toast.Provider>
    </LabelsContext.Provider>
  )
}

function AnchoredToasts({ activeScope }: { activeScope: string | null }) {
  const { toasts } = Toast.useToastManager<ToastData>()

  return (
    <Toast.Portal data-slot='toast-portal-anchored'>
      <Toast.Viewport
        className='outline-none'
        data-slot='toast-viewport-anchored'
      >
        {toasts
          .filter((toast) => shouldRenderScopedToast(toast.data, activeScope))
          .map((toast) => {
            const anchor = toast.positionerProps?.anchor
            if (!anchor) return null
            const tooltipStyle = toast.data?.tooltipStyle ?? false
            const body = describeToastBody(toast)
            return (
              <Toast.Positioner
                key={toast.id}
                toast={toast}
                sideOffset={toast.positionerProps?.sideOffset ?? 4}
                className='z-100 max-w-[min(--spacing(64),var(--available-width))]'
                data-slot='toast-positioner'
              >
                <Toast.Root
                  toast={toast}
                  data-slot='toast-popup'
                  className={cn(
                    TOAST_SURFACE,
                    'relative overflow-visible text-xs text-balance transition-[scale,opacity] data-ending-style:scale-98 data-ending-style:opacity-0 data-starting-style:scale-98 data-starting-style:opacity-0',
                    tooltipStyle ? 'rounded-md' : 'rounded-lg'
                  )}
                >
                  {tooltipStyle ? (
                    <Toast.Content className='pointer-events-auto px-2 py-1'>
                      <Toast.Title data-slot='toast-title' />
                    </Toast.Content>
                  ) : (
                    <>
                      <DismissButton
                        manager={anchoredToastManager}
                        toastId={toast.id}
                        onClose={toast.data?.onClose}
                      />
                      <Toast.Content
                        className={cn(
                          'pointer-events-auto min-h-0 [overflow-x:clip] overflow-y-visible pl-3.5 text-sm',
                          body.stackedActionLayout
                            ? 'flex flex-col gap-2 py-2.5 pr-3.5'
                            : cn(
                                'flex items-center justify-between gap-1.5 py-3',
                                body.inlineContentEndPad
                              )
                        )}
                      >
                        <ToastBodyContent
                          {...body}
                          actionProps={toast.actionProps}
                          toastData={toast.data}
                          toastDescription={toast.description}
                          toastType={toast.type}
                        />
                      </Toast.Content>
                    </>
                  )}
                </Toast.Root>
              </Toast.Positioner>
            )
          })}
      </Toast.Viewport>
    </Toast.Portal>
  )
}

export {
  AnchoredToastProvider,
  anchoredToastManager,
  ToastProvider,
  toastManager
}
