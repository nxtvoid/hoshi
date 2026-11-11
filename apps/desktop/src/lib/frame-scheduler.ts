type Options = {
  /** How long to wait when no frame comes, as in a hidden window. */
  fallbackMs?: number
  /**
   * The most of the thread's time the runs may take, from 0 to 1. After a run whose frame
   * took long, the next one waits, so the rest stays for whatever else the user does.
   */
  share?: number
}

/** However slow a run was, the next one waits no longer than this. */
const LONGEST_REST_MS = 250

const now = () => performance.now()

/**
 * Runs `run` once on the next animation frame however often it is scheduled, or after
 * `fallbackMs` when no frame comes.
 */
export function frameScheduler(
  run: () => void,
  { fallbackMs = 50, share = 1 }: Options = {}
) {
  let frame: number | null = null
  let timer: ReturnType<typeof setTimeout> | null = null
  let rest: ReturnType<typeof setTimeout> | null = null
  let restUntil = 0
  let measuring = false
  let wanted = false

  const cancel = () => {
    if (frame !== null) cancelAnimationFrame(frame)
    if (timer !== null) clearTimeout(timer)
    if (rest !== null) clearTimeout(rest)
    frame = null
    timer = null
    rest = null
    wanted = false
  }
  const fire = () => {
    cancel()
    const started = now()
    run()
    if (share >= 1) return
    measuring = true
    // A timer runs once the frame is rendered, so this takes in layout and paint.
    setTimeout(() => {
      measuring = false
      const cost = now() - started
      restUntil = started + Math.min(cost / share, LONGEST_REST_MS)
      if (wanted) schedule()
    }, 0)
  }
  const request = () => {
    if (typeof requestAnimationFrame === 'function') {
      frame = requestAnimationFrame(fire)
    }
    timer = setTimeout(fire, fallbackMs)
  }
  const schedule = () => {
    if (timer !== null || rest !== null) return
    if (measuring) {
      wanted = true
      return
    }
    wanted = false
    const wait = restUntil - now()
    if (wait <= 0) return request()
    rest = setTimeout(() => {
      rest = null
      request()
    }, wait)
  }
  return { schedule, cancel }
}
