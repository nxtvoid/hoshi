/** Base UI updates leave out `undefined`; an action cleared with empty children is hidden. */
export function hasVisibleToastAction(actionProps: unknown): boolean {
  if (actionProps == null || typeof actionProps !== 'object') return false
  if (!('children' in actionProps)) return false
  const children = actionProps.children
  return children != null && children !== false && children !== ''
}

/** In a collapsed stack only the frontmost toast shows its content. */
export function shouldHideCollapsedToastContent(
  visibleToastIndex: number,
  visibleToastCount: number
): boolean {
  if (visibleToastCount <= 1) return false
  return visibleToastIndex > 0
}

type ToastLayoutProps = {
  height?: number | null | undefined
  transitionStatus?: 'starting' | 'ending' | undefined
}

type VisibleToastLayoutItem<TToast> = {
  toast: TToast
  visibleIndex: number
  offsetY: number
}

/**
 * Where each toast of a stack goes. A toast on its way out keeps its place, so the others
 * don't jump; the live ones close up behind it.
 */
export function buildVisibleToastLayout<TToast extends ToastLayoutProps>(
  visibleToasts: readonly TToast[]
): { frontmostHeight: number; items: VisibleToastLayoutItem<TToast>[] } {
  let fullIndex = 0
  let fullOffsetY = 0
  let liveIndex = 0
  let liveOffsetY = 0

  const items = visibleToasts.map((toast) => {
    const height = normalizedHeight(toast.height)
    const item =
      toast.transitionStatus === 'ending'
        ? { toast, visibleIndex: fullIndex, offsetY: fullOffsetY }
        : { toast, visibleIndex: liveIndex, offsetY: liveOffsetY }
    fullOffsetY += height
    fullIndex += 1
    if (toast.transitionStatus !== 'ending') {
      liveOffsetY += height
      liveIndex += 1
    }
    return item
  })

  const frontmost = visibleToasts.find(
    (toast) => toast.transitionStatus !== 'ending'
  )
  return { frontmostHeight: normalizedHeight(frontmost?.height), items }
}

function normalizedHeight(height: number | null | undefined): number {
  return typeof height === 'number' && Number.isFinite(height) && height > 0
    ? height
    : 0
}

/** A toast with a scope shows only while that scope, such as a thread, is the active one. */
export function shouldRenderScopedToast(
  data: { scope?: string | null } | undefined,
  activeScope: string | null
): boolean {
  const scope = data?.scope
  if (!scope) return true
  return activeScope === scope
}
