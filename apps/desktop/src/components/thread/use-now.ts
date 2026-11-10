import { useEffect, useState } from 'react'

/** The current time, read again every second while `ticking`. */
export function useNow(ticking = true) {
  const [now, setNow] = useState(Date.now)
  useEffect(() => {
    if (!ticking) return
    setNow(Date.now())
    const timer = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(timer)
  }, [ticking])
  return now
}
