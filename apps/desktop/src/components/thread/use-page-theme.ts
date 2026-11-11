import { useSyncExternalStore } from 'react'
import { type PageTheme, pageTheme } from '@/lib/html-page'

let current: PageTheme | null = null
let observer: MutationObserver | null = null
const listeners = new Set<() => void>()

function read(): PageTheme {
  const root = document.documentElement
  const style = getComputedStyle(root)
  return pageTheme(root.classList.contains('dark'), (name) =>
    style.getPropertyValue(name)
  )
}

function subscribe(listener: () => void) {
  listeners.add(listener)
  if (!observer) {
    observer = new MutationObserver(() => {
      const next = read()
      if (JSON.stringify(next) === JSON.stringify(current)) return
      current = next
      for (const notify of listeners) notify()
    })
    observer.observe(document.documentElement, { attributes: true })
  }
  return () => {
    listeners.delete(listener)
    if (listeners.size > 0) return
    observer?.disconnect()
    observer = null
    current = null
  }
}

function snapshot() {
  current ??= read()
  return current
}

/** The theme on screen, for the pages agents show; changes with the app's. */
export function usePageTheme() {
  return useSyncExternalStore(subscribe, snapshot)
}
