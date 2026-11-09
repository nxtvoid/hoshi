/**
 * Runs `run` once on the next animation frame however often it is scheduled, or after
 * `fallbackMs` when no frame comes, as in a hidden window.
 */
export function frameScheduler(run: () => void, fallbackMs = 50) {
  let frame: number | null = null
  let timer: ReturnType<typeof setTimeout> | null = null
  const cancel = () => {
    if (frame !== null) cancelAnimationFrame(frame)
    if (timer !== null) clearTimeout(timer)
    frame = null
    timer = null
  }
  const fire = () => {
    cancel()
    run()
  }
  return {
    schedule: () => {
      if (timer !== null) return
      if (typeof requestAnimationFrame === 'function') {
        frame = requestAnimationFrame(fire)
      }
      timer = setTimeout(fire, fallbackMs)
    },
    cancel
  }
}
