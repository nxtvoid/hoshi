import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { invoke } from '@tauri-apps/api/core'
import { HugeiconsIcon } from '@hugeicons/react'
import { Loading03Icon, StopIcon } from '@hugeicons/core-free-icons'
import { getLocale } from '@/paraglide/runtime'
import { useServerStore } from '@/state/server-store'
import type { LocalServer } from '@/bindings/LocalServer'
import type { CommandError } from '@/bindings/CommandError'
import { useEffect, useState } from 'react'
import { useBackgroundTaskStore } from '@/state/background-task-store'
import {
  ageTick,
  SERVERS_SHOWN,
  serverDetail,
  stopsFromRow
} from '@/lib/servers'

const NO_SERVERS: LocalServer[] = []

const ROW = 'flex min-h-10 min-w-0 items-center gap-2.5 px-2.5 py-1'

function serverKey(server: LocalServer) {
  return `${server.pid}:${server.port}`
}

function stopError(error: CommandError) {
  switch (error.code) {
    case 'serverNotFound':
      return m.error_server_gone()
    case 'serverStopRefused':
      return m.error_server_stop_refused()
    case 'serverStopFailed':
      return error.detail
    default:
      return m.error_generic()
  }
}

function useDocumentVisible() {
  const [visible, setVisible] = useState(
    () => document.visibilityState === 'visible'
  )
  useEffect(() => {
    const onChange = () => setVisible(document.visibilityState === 'visible')
    document.addEventListener('visibilitychange', onChange)
    return () => document.removeEventListener('visibilitychange', onChange)
  }, [])
  return visible
}

/** Asks the engine to scan for the thread's servers while this is mounted and seen. */
export function useWatchedServers(threadId: string) {
  const visible = useDocumentVisible()
  const receive = useServerStore((state) => state.receive)
  useEffect(() => {
    if (!visible) return
    let current = true
    const watching = invoke<LocalServer[]>('watch_servers', { threadId })
      .then((servers) => {
        if (current) receive({ threadId, servers })
      })
      .catch(() => {})
    return () => {
      current = false
      void watching.then(() =>
        invoke('unwatch_servers', { threadId }).catch(() => {})
      )
    }
  }, [threadId, visible, receive])
}

/** The time the servers' ages are told against, moving on while any is listed. */
function useNow(servers: LocalServer[]) {
  const [now, setNow] = useState(Date.now)
  const every = servers.length === 0 ? null : ageTick(servers, now)
  useEffect(() => {
    if (every === null) return
    setNow(Date.now())
    const timer = setInterval(() => setNow(Date.now()), every)
    return () => clearInterval(timer)
  }, [every])
  return now
}

/**
 * In the thread's header while its details are out of sight: how many servers it has
 * running, which opens the details that list them. Nothing while it has none.
 */
export function ServersNotice({
  threadId,
  onOpen
}: {
  threadId: string
  onOpen: () => void
}) {
  useWatchedServers(threadId)
  const count = useServerStore((state) => state.byThread[threadId]?.length ?? 0)
  if (count === 0) return null
  return (
    <button
      type='button'
      onClick={onOpen}
      className='border-border text-muted-foreground hover:bg-foreground/5 hover:text-foreground focus-visible:ring-ring flex h-6.5 shrink-0 cursor-pointer items-center gap-1.5 rounded-md border px-2 text-xs whitespace-nowrap outline-none focus-visible:ring-2'
    >
      <span className='bg-status-done size-1.75 shrink-0 rounded-full' />
      {count === 1 ? m.servers_running_one() : m.servers_running({ count })}
    </button>
  )
}

/** The localhost servers the thread's agent and terminals started; hidden while none run. */
export function ThreadServers({ threadId }: { threadId: string }) {
  useWatchedServers(threadId)
  const servers = useServerStore(
    (state) => state.byThread[threadId] ?? NO_SERVERS
  )
  const [expanded, setExpanded] = useState(false)
  const [stopping, setStopping] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const commandsRunning = useBackgroundTaskStore(
    (state) => state.byThread[threadId]?.tasks.length ?? 0
  )
  const now = useNow(servers)

  if (servers.length === 0) return null
  const shown = expanded ? servers : servers.slice(0, SERVERS_SHOWN)
  const hidden = servers.length - SERVERS_SHOWN

  const stop = async (server: LocalServer) => {
    if (stopping) return
    setStopping(serverKey(server))
    setError(null)
    try {
      await invoke('stop_server', {
        threadId,
        pid: server.pid,
        port: server.port
      })
    } catch (failure) {
      setError(stopError(failure as CommandError))
    } finally {
      setStopping(null)
    }
  }

  return (
    <>
      <div className='bg-border/70 mx-2.5 my-1 h-px' />
      <p className='text-muted-foreground px-2.5 pt-1 pb-0.5 text-xs'>
        {m.servers()}
      </p>
      <ul>
        {shown.map((server) => {
          const key = serverKey(server)
          const origin =
            server.origin === 'terminal'
              ? m.server_origin_terminal()
              : m.server_origin_agent()
          return (
            <li key={key} className='relative'>
              <div
                className={cn(
                  ROW,
                  stopsFromRow(server, commandsRunning) && 'pr-10'
                )}
              >
                <span className='bg-status-done size-1.75 shrink-0 rounded-full' />
                <span className='flex min-w-0 flex-1 flex-col'>
                  <span className='text-foreground/80 truncate text-[13px] font-medium'>
                    localhost:{server.port}
                  </span>
                  <span className='text-muted-foreground truncate text-xs'>
                    {serverDetail(server, origin, now, getLocale())}
                  </span>
                </span>
              </div>
              {stopsFromRow(server, commandsRunning) ? (
                <button
                  type='button'
                  aria-label={m.server_stop_label({ port: server.port })}
                  title={m.server_stop_label({ port: server.port })}
                  disabled={stopping !== null}
                  onClick={() => void stop(server)}
                  className='text-destructive hover:bg-destructive/10 focus-visible:ring-destructive/40 absolute top-1/2 right-1.5 grid size-6 -translate-y-1/2 cursor-pointer place-items-center rounded-md outline-none focus-visible:ring-2 disabled:cursor-default'
                >
                  <HugeiconsIcon
                    icon={stopping === key ? Loading03Icon : StopIcon}
                    strokeWidth={2}
                    className={cn(
                      'size-3.5',
                      stopping === key && 'animate-spin'
                    )}
                  />
                </button>
              ) : null}
            </li>
          )
        })}
      </ul>
      {hidden > 0 ? (
        <button
          type='button'
          aria-expanded={expanded}
          onClick={() => setExpanded((value) => !value)}
          className='text-muted-foreground hover:bg-foreground/5 focus-visible:ring-ring flex h-8 w-full cursor-pointer items-center rounded-lg px-2.5 text-left text-xs outline-none focus-visible:ring-2 focus-visible:ring-inset'
        >
          {expanded
            ? m.servers_show_less()
            : m.servers_show_more({ count: hidden })}
        </button>
      ) : null}
      {error ? (
        <p role='alert' className='text-destructive px-2.5 pt-1 pb-1 text-xs'>
          {error}
        </p>
      ) : null}
    </>
  )
}
