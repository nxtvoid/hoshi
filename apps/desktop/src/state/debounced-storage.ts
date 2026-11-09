import type { PersistStorage, StorageValue } from 'zustand/middleware'

type Backing = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>

export type DebouncedStorage<S> = PersistStorage<S> & {
  /** Writes what is waiting now. */
  flush: () => void
}

/**
 * JSON storage for `persist` that writes at most once every `delayMs`, so a store that
 * changes on every keystroke doesn't serialize itself on every keystroke.
 */
export function debouncedStorage<S>(
  delayMs: number,
  backing: () => Backing = () => localStorage
): DebouncedStorage<S> {
  const waiting = new Map<string, StorageValue<S>>()
  let timer: ReturnType<typeof setTimeout> | null = null
  const flush = () => {
    if (timer !== null) clearTimeout(timer)
    timer = null
    for (const [name, value] of waiting) {
      try {
        backing().setItem(name, JSON.stringify(value))
      } catch (error) {
        console.error('hoshi: failed to save', name, error)
      }
    }
    waiting.clear()
  }
  return {
    getItem: (name) => {
      const value = waiting.get(name)
      if (value) return value
      try {
        const text = backing().getItem(name)
        return text === null ? null : (JSON.parse(text) as StorageValue<S>)
      } catch {
        return null
      }
    },
    setItem: (name, value) => {
      waiting.set(name, value)
      timer ??= setTimeout(flush, delayMs)
    },
    removeItem: (name) => {
      waiting.delete(name)
      backing().removeItem(name)
    },
    flush
  }
}

/** Writes what waits before the window hides or closes. */
export function flushOnLeave(flush: () => void) {
  if (typeof window === 'undefined') return
  window.addEventListener('beforeunload', flush)
  window.addEventListener('pagehide', flush)
  window.addEventListener('blur', flush)
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'hidden') flush()
  })
}
