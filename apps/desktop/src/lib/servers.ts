import type { LocalServer } from '@/bindings/LocalServer'

/** Rows shown before the rest folds behind "Show more". */
export const SERVERS_SHOWN = 4

const AGE_UNITS = [
  ['day', 86_400],
  ['hour', 3_600],
  ['minute', 60],
  ['second', 1]
] as const

const ageFormatters = new Map<string, Intl.NumberFormat>()

/** How long a server has run, in its largest whole unit, such as "12 min". */
export function serverAge(startedAt: number, now: number, locale: string) {
  const seconds = Math.max(0, Math.floor((now - startedAt) / 1000))
  const [unit, size] =
    AGE_UNITS.find(([, size]) => seconds >= size) ?? AGE_UNITS[3]
  const key = `${locale}:${unit}`
  let formatter = ageFormatters.get(key)
  if (!formatter) {
    formatter = new Intl.NumberFormat(locale, {
      style: 'unit',
      unit,
      unitDisplay: 'short'
    })
    ageFormatters.set(key, formatter)
  }
  return formatter.format(Math.floor(seconds / size))
}

/** What started it, its process and its age, leaving out what is unknown. */
export function serverDetail(
  server: LocalServer,
  origin: string,
  now: number,
  locale: string
) {
  return [
    origin,
    server.processName,
    server.startedAt === null ? null : serverAge(server.startedAt, now, locale)
  ]
    .filter(Boolean)
    .join(' · ')
}

/** How often a server's age is told again: each second in its first minute. */
export function ageTick(servers: LocalServer[], now: number) {
  const young = servers.some(
    (server) => server.startedAt !== null && now - server.startedAt < 60_000
  )
  return young ? 1_000 : 30_000
}

/**
 * Whether a server is stopped from its row. While the agent runs commands in the
 * background, one of them may be the server: ending it under the agent would have the
 * agent take the command as failed and start it again, so the command is stopped where
 * the transcript shows it running.
 */
export function stopsFromRow(server: LocalServer, commandsRunning: number) {
  return server.origin === 'terminal' || commandsRunning === 0
}
