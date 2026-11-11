import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { frameScheduler } from '../../src/lib/frame-scheduler'

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))

const globals = globalThis as {
  requestAnimationFrame?: (callback: () => void) => number
  cancelAnimationFrame?: (id: number) => void
}

const { requestAnimationFrame, cancelAnimationFrame } = globals

describe('frameScheduler', () => {
  beforeEach(() => {
    delete globals.requestAnimationFrame
    delete globals.cancelAnimationFrame
  })

  afterEach(() => {
    globals.requestAnimationFrame = requestAnimationFrame
    globals.cancelAnimationFrame = cancelAnimationFrame
  })

  test('runs once however often it is scheduled', async () => {
    let runs = 0
    const scheduler = frameScheduler(
      () => {
        runs += 1
      },
      { fallbackMs: 5 }
    )
    scheduler.schedule()
    scheduler.schedule()
    scheduler.schedule()
    await wait(20)
    expect(runs).toBe(1)
    scheduler.schedule()
    await wait(20)
    expect(runs).toBe(2)
  })

  test('does not run once cancelled', async () => {
    let runs = 0
    const scheduler = frameScheduler(
      () => {
        runs += 1
      },
      { fallbackMs: 5 }
    )
    scheduler.schedule()
    scheduler.cancel()
    await wait(20)
    expect(runs).toBe(0)
  })

  test('runs on the next frame and skips the fallback', async () => {
    const frames: (() => void)[] = []
    globals.requestAnimationFrame = (callback) => frames.push(callback)
    globals.cancelAnimationFrame = () => {}
    let runs = 0
    const scheduler = frameScheduler(
      () => {
        runs += 1
      },
      { fallbackMs: 5 }
    )
    scheduler.schedule()
    expect(frames).toHaveLength(1)
    frames[0]?.()
    expect(runs).toBe(1)
    await wait(20)
    expect(runs).toBe(1)
  })

  test('rests after a slow run, so the runs take no more than their share', async () => {
    const block = (ms: number) => {
      const until = performance.now() + ms
      while (performance.now() < until);
    }
    let runs = 0
    const scheduler = frameScheduler(
      () => {
        runs += 1
        block(30)
      },
      { fallbackMs: 1, share: 0.25 }
    )
    scheduler.schedule()
    await wait(10)
    expect(runs).toBe(1)
    const asking = setInterval(scheduler.schedule, 2)
    await wait(60)
    expect(runs).toBe(1)
    await wait(150)
    clearInterval(asking)
    scheduler.cancel()
    expect(runs).toBe(2)
  })

  test('a quick run is followed at once', async () => {
    let runs = 0
    const scheduler = frameScheduler(
      () => {
        runs += 1
      },
      { fallbackMs: 1, share: 0.25 }
    )
    scheduler.schedule()
    await wait(10)
    scheduler.schedule()
    await wait(20)
    expect(runs).toBe(2)
  })
})
