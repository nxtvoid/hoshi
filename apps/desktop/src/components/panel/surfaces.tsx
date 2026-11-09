import { m } from '@/paraglide/messages'
import { DiffSurface } from './DiffSurface'
import { BrowserSurface } from './BrowserSurface'
import { useAppStore } from '@/state/app-store'
import type { ComponentType, ReactNode } from 'react'
import { usePanelStore } from '@/state/panel-store'
import { PullRequestSurface } from './pull-request/PullRequestSurface'
import { LazyTerminalView } from '@/components/terminal/LazyTerminalView'
import {
  ComputerTerminal01Icon,
  FileDiffIcon,
  GitPullRequestIcon,
  Globe02Icon
} from '@hugeicons/core-free-icons'
import type { Surface, SurfaceKind } from '@/state/panel-surfaces'

/** What decides whether a kind of surface can open for a thread. */
export type SurfaceContext = { hasPullRequest: boolean }

type SurfaceDefinition<S extends Surface> = {
  label: () => string
  icon: typeof FileDiffIcon
  /** Single key that opens it from the launcher. */
  shortcut: string
  /** Omitted when it can always open. */
  available?: (context: SurfaceContext) => boolean
  /** The tab's text; the label when omitted. */
  Title?: ComponentType<{ threadId: string }>
  render: (threadId: string, surface: S) => ReactNode
}

/**
 * Every kind of surface the panel can show. A new kind (terminal, files, browser,
 * side chat) is one entry here plus its state in `panel-surfaces`.
 */
export const SURFACES: {
  [K in SurfaceKind]: SurfaceDefinition<Extract<Surface, { kind: K }>>
} = {
  terminal: {
    label: m.surface_terminal,
    icon: ComputerTerminal01Icon,
    shortcut: 'T',
    render: (threadId, surface) => (
      <TerminalSurface threadId={threadId} terminalId={surface.id} />
    )
  },
  diff: {
    label: m.surface_diff,
    icon: FileDiffIcon,
    shortcut: 'D',
    render: (threadId, surface) => (
      <DiffSurface threadId={threadId} surface={surface} />
    )
  },
  pullRequest: {
    label: m.surface_pull_request,
    icon: GitPullRequestIcon,
    shortcut: 'P',
    available: (context) => context.hasPullRequest,
    Title: PullRequestTitle,
    render: (threadId) => <PullRequestSurface threadId={threadId} />
  },
  browser: {
    label: m.surface_browser,
    icon: Globe02Icon,
    shortcut: 'B',
    render: (threadId) => <BrowserSurface threadId={threadId} />
  }
}

/** Launcher order. */
export const SURFACE_KINDS: SurfaceKind[] = [
  'terminal',
  'diff',
  'pullRequest',
  'browser'
]

export function useSurfaceContext(threadId: string): SurfaceContext {
  const hasPullRequest = useAppStore(
    (state) => state.pullRequests[threadId] !== undefined
  )
  return { hasPullRequest }
}

export function isAvailable(kind: SurfaceKind, context: SurfaceContext) {
  return SURFACES[kind].available?.(context) ?? true
}

export function renderSurface(threadId: string, surface: Surface): ReactNode {
  switch (surface.kind) {
    case 'diff':
      return SURFACES.diff.render(threadId, surface)
    case 'pullRequest':
      return SURFACES.pullRequest.render(threadId, surface)
    case 'terminal':
      return SURFACES.terminal.render(threadId, surface)
    case 'browser':
      return SURFACES.browser.render(threadId, surface)
  }
}

/** The available kind whose launcher shortcut is `key`, ignoring case. */
export function kindForShortcut(
  key: string,
  context: SurfaceContext
): SurfaceKind | undefined {
  return SURFACE_KINDS.find(
    (kind) =>
      SURFACES[kind].shortcut.toLowerCase() === key.toLowerCase() &&
      isAvailable(kind, context)
  )
}

function PullRequestTitle({ threadId }: { threadId: string }) {
  const number = useAppStore((state) => state.pullRequests[threadId]?.number)
  return <>{number === undefined ? m.surface_pull_request() : `#${number}`}</>
}

function TerminalSurface({
  threadId,
  terminalId
}: {
  threadId: string
  terminalId: string
}) {
  const openSurface = usePanelStore((state) => state.openSurface)
  return (
    <LazyTerminalView
      threadId={threadId}
      terminalId={terminalId}
      onOpenHere={() => openSurface(threadId, 'terminal')}
    />
  )
}
