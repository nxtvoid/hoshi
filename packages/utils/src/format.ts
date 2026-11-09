const UNITS = [
  ['year', 31_536_000],
  ['month', 2_592_000],
  ['week', 604_800],
  ['day', 86_400],
  ['hour', 3_600],
  ['minute', 60]
] as const

export function relativeTimeParts(
  then: Date,
  now: Date
): { value: number; unit: Intl.RelativeTimeFormatUnit } {
  const seconds = Math.round((then.getTime() - now.getTime()) / 1000)
  for (const [unit, size] of UNITS) {
    if (Math.abs(seconds) >= size) {
      return { value: Math.trunc(seconds / size), unit }
    }
  }
  return { value: 0, unit: 'second' }
}

const formatters = new Map<string, Intl.RelativeTimeFormat>()

export function formatRelativeTime(iso: string, now: Date, locale: string) {
  let formatter = formatters.get(locale)
  if (!formatter) {
    formatter = new Intl.RelativeTimeFormat(locale, {
      numeric: 'auto',
      style: 'narrow'
    })
    formatters.set(locale, formatter)
  }
  const { value, unit } = relativeTimeParts(new Date(iso), now)
  return formatter.format(value, unit)
}

const DURATION_UNITS = [
  ['hour', 3_600],
  ['minute', 60],
  ['second', 1]
] as const

/** A short duration such as "1m 12s", its two largest units, in `locale`. */
export function formatDuration(milliseconds: number, locale: string) {
  let rest = Math.max(0, Math.round(milliseconds / 1000))
  const parts: string[] = []
  for (const [unit, size] of DURATION_UNITS) {
    const value = Math.floor(rest / size)
    rest -= value * size
    if (value === 0 && !(unit === 'second' && parts.length === 0)) continue
    parts.push(
      new Intl.NumberFormat(locale, {
        style: 'unit',
        unit,
        unitDisplay: 'narrow'
      }).format(value)
    )
  }
  return parts.slice(0, 2).join(' ')
}

const SIZE_UNITS = [
  ['gigabyte', 1_000_000_000],
  ['megabyte', 1_000_000],
  ['kilobyte', 1_000]
] as const

/** A file size such as "8 kB" or "1.4 MB", in `locale`. */
export function formatFileSize(bytes: number, locale: string) {
  const [unit, size] = SIZE_UNITS.find(([, size]) => bytes >= size) ?? [
    'byte',
    1
  ]
  return new Intl.NumberFormat(locale, {
    style: 'unit',
    unit,
    unitDisplay: 'short',
    maximumFractionDigits: bytes >= 1_000_000 ? 1 : 0
  }).format(bytes / size)
}

/** The time of day of `iso`, as "22:10" or "10:10 PM" in `locale`. */
export function formatClockTime(
  iso: string,
  locale: string,
  timeZone?: string
) {
  const date = new Date(iso)
  if (Number.isNaN(date.getTime())) return ''
  return new Intl.DateTimeFormat(locale, {
    hour: '2-digit',
    minute: '2-digit',
    timeZone
  }).format(date)
}
