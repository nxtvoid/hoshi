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
