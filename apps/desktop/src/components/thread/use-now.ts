import { useEffect, useState, useSyncExternalStore } from 'react'

const listeners = new Set<() => void>()
let now = Date.now()
let timer: ReturnType<typeof setInterval> | undefined

/** One clock for every ticking reader, so they all redraw in the same frame each second. */
function subscribe(listener: () => void) {
  listeners.add(listener)
  if (timer === undefined) {
    now = Date.now()
    timer = setInterval(() => {
      now = Date.now()
      for (const notify of listeners) notify()
    }, 1000)
  }
  return () => {
    listeners.delete(listener)
    if (listeners.size > 0) return
    clearInterval(timer)
    timer = undefined
  }
}

const read = () => now
const still = () => () => {}

/** The current time, read again every second while `ticking`; when it stops, as it stopped. */
export function useNow(ticking = true) {
  const shared = useSyncExternalStore(ticking ? subscribe : still, read)
  const [stopped, setStopped] = useState(Date.now)
  useEffect(() => {
    if (!ticking) setStopped(Date.now())
  }, [ticking])
  return ticking ? shared : stopped
}
