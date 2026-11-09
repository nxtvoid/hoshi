import '@xterm/xterm/css/xterm.css'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { m } from '@/paraglide/messages'
import { CODE_SIZE } from '@/state/appearance'
import { WebglAddon } from '@xterm/addon-webgl'
import { useAppStore } from '@/state/app-store'
import { HugeiconsIcon } from '@hugeicons/react'
import { useEffect, useRef, useState } from 'react'
import { Channel, invoke } from '@tauri-apps/api/core'
import { useTerminalStore } from '@/state/terminal-store'
import { Button } from '@hoshi/ui/components/button'
import type { TerminalExit } from '@/bindings/TerminalExit'
import type { CommandError } from '@/bindings/CommandError'
import type { TerminalEvent } from '@/bindings/TerminalEvent'
import type { TerminalAttach } from '@/bindings/TerminalAttach'
import { GitForkIcon } from '@hugeicons/core-free-icons'

/** Keys the app handles even while a terminal has the keyboard. */
function isAppShortcut(event: KeyboardEvent) {
  return event.ctrlKey && !event.altKey && event.code === 'KeyJ'
}

function readTheme(element: HTMLElement) {
  const style = getComputedStyle(element)
  const token = (name: string) => style.getPropertyValue(name).trim()
  return {
    background: token('--background'),
    foreground: token('--foreground'),
    cursor: token('--foreground'),
    cursorAccent: token('--background'),
    selectionBackground: token('--terminal-selection')
  }
}

function readFont(element: HTMLElement) {
  const style = getComputedStyle(element)
  return {
    fontFamily: style.getPropertyValue('--font-terminal').trim() || 'monospace',
    fontSize:
      Number.parseFloat(style.getPropertyValue('--terminal-font-size')) ||
      CODE_SIZE.default
  }
}

function exitNotice(exit: TerminalExit) {
  const text =
    exit.code === null
      ? m.terminal_exited()
      : m.terminal_exited_with_code({ code: exit.code })
  return `\r\n\x1b[2m${text}\x1b[0m\r\n`
}

function errorMessage(error: CommandError) {
  return error.code === 'terminalFailed' ? error.detail : m.error_generic()
}

/** The folder a thread's agent works in: its worktree when it has one. */
function useThreadCwd(threadId: string) {
  return useAppStore((state) => {
    const thread = state.threads.find((candidate) => candidate.id === threadId)
    if (!thread) return null
    return (
      thread.worktreePath ??
      state.projects.find((project) => project.id === thread.projectId)
        ?.workspaceRoot ??
      null
    )
  })
}

/**
 * A thread's shell. Unmounting only detaches the view; the shell keeps running until
 * its tab is closed.
 */
export function TerminalView({
  threadId,
  terminalId,
  onOpenHere
}: {
  threadId: string
  terminalId: string
  /** Opens another terminal, which starts where the thread works now. */
  onOpenHere: () => void
}) {
  const containerRef = useRef<HTMLDivElement>(null)
  const terminalRef = useRef<Terminal | null>(null)
  const [startedIn, setStartedIn] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const threadCwd = useThreadCwd(threadId)
  const wantsFocus = useTerminalStore(
    (state) => state.pendingFocus === terminalId
  )
  const clearFocus = useTerminalStore((state) => state.clearFocus)

  useEffect(() => {
    const container = containerRef.current
    if (!container) return
    const terminal = new Terminal({
      ...readFont(container),
      lineHeight: 1.2,
      cursorBlink: true,
      scrollback: 5000,
      theme: readTheme(container)
    })
    const fit = new FitAddon()
    terminal.loadAddon(fit)
    terminal.attachCustomKeyEventHandler((event) => !isAppShortcut(event))
    terminal.open(container)
    try {
      const webgl = new WebglAddon()
      webgl.onContextLoss(() => webgl.dispose())
      terminal.loadAddon(webgl)
    } catch {
      // The DOM renderer stays in use.
    }
    terminalRef.current = terminal
    const fitVisible = () => {
      if (container.clientWidth > 0 && container.clientHeight > 0) fit.fit()
    }
    fitVisible()

    let disposed = false
    let exited = false

    const attach = () => {
      const pending: TerminalEvent[] = []
      let ready = false
      const handle = (event: TerminalEvent) => {
        if (event.type === 'output') {
          terminal.write(event.data)
        } else {
          exited = true
          terminal.write(exitNotice(event.exit))
        }
      }
      const channel = new Channel<TerminalEvent>()
      channel.onmessage = (event) => {
        if (disposed) return
        if (ready) handle(event)
        else pending.push(event)
      }
      invoke<TerminalAttach>('open_terminal', {
        threadId,
        terminalId,
        cols: terminal.cols,
        rows: terminal.rows,
        onEvent: channel
      })
        .then((attached) => {
          if (disposed) return
          setStartedIn(attached.cwd)
          setError(null)
          terminal.write(attached.history)
          if (attached.exit) {
            exited = true
            terminal.write(exitNotice(attached.exit))
          }
          ready = true
          for (const event of pending.splice(0)) handle(event)
        })
        .catch((failure: CommandError) => {
          if (!disposed) setError(errorMessage(failure))
        })
    }
    attach()

    const input = terminal.onData((data) => {
      if (exited) {
        exited = false
        terminal.reset()
        void invoke('close_terminal', { terminalId }).then(attach)
        return
      }
      void invoke('write_terminal', { terminalId, data }).catch(() => {})
    })
    const resize = terminal.onResize(({ cols, rows }) => {
      void invoke('resize_terminal', { terminalId, cols, rows }).catch(() => {})
    })
    const sizeObserver = new ResizeObserver(fitVisible)
    sizeObserver.observe(container)
    const themeObserver = new MutationObserver(() => {
      terminal.options.theme = readTheme(container)
      const font = readFont(container)
      if (
        font.fontFamily !== terminal.options.fontFamily ||
        font.fontSize !== terminal.options.fontSize
      ) {
        terminal.options.fontFamily = font.fontFamily
        terminal.options.fontSize = font.fontSize
        fitVisible()
      }
    })
    themeObserver.observe(document.documentElement, {
      attributes: true,
      attributeFilter: [
        'class',
        'style',
        'data-palette',
        'data-code-font',
        'data-terminal-font'
      ]
    })

    return () => {
      disposed = true
      sizeObserver.disconnect()
      themeObserver.disconnect()
      input.dispose()
      resize.dispose()
      terminal.dispose()
      terminalRef.current = null
      void invoke('detach_terminal', { terminalId })
    }
  }, [threadId, terminalId])

  useEffect(() => {
    if (!wantsFocus) return
    terminalRef.current?.focus()
    clearFocus()
  }, [wantsFocus, clearFocus])

  const moved =
    startedIn !== null && threadCwd !== null && startedIn !== threadCwd
  const folder = threadCwd?.split(/[\\/]/).filter(Boolean).pop() ?? ''

  return (
    <div className='bg-background flex min-h-0 flex-1 flex-col'>
      {moved ? (
        <div className='border-border text-muted-foreground flex shrink-0 items-center gap-2 border-b px-3 py-1.5 text-xs'>
          <HugeiconsIcon
            icon={GitForkIcon}
            strokeWidth={2}
            className='size-3.5 shrink-0'
          />
          <span className='min-w-0 flex-1 truncate'>
            {m.terminal_thread_moved({ folder })}
          </span>
          <Button size='xs' variant='outline' onClick={onOpenHere}>
            {m.open_terminal_here()}
          </Button>
        </div>
      ) : null}
      {error ? (
        <p role='alert' className='text-destructive shrink-0 px-3 py-2 text-xs'>
          {error}
        </p>
      ) : null}
      <div className='min-h-0 flex-1 py-1 pl-2'>
        <div ref={containerRef} className='size-full' />
      </div>
    </div>
  )
}
