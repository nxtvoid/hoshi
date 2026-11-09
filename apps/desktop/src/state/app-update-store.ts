import { create } from 'zustand'
import { relaunch } from '@tauri-apps/plugin-process'
import { getVersion } from '@tauri-apps/api/app'
import { check, type Update } from '@tauri-apps/plugin-updater'
import { type AppUpdate, checkDue, downloadPercent } from '@/lib/updates'

const CHECK_EVERY = 30 * 60 * 1000
const UP_TO_DATE_FOR = 2000

/** Dev builds never update. */
export const APP_UPDATES_ENABLED = import.meta.env.PROD

type AppUpdateState = {
  update: AppUpdate
  /** The running build's version; `null` until read. */
  currentVersion: string | null
  /** Reads the release feed; `manual` shows the check and how it ended. */
  check: (manual?: boolean) => Promise<void>
  /** Downloads the update found last; `restart` installs it. */
  download: () => Promise<void>
  restart: () => Promise<void>
}

/** The plugin's handle on the release found last; it can't live in the store. */
let pending: Update | null = null
let checking: Promise<void> | null = null
let lastCheckedAt: number | null = null
let upToDateTimer: ReturnType<typeof setTimeout> | undefined

function busy(update: AppUpdate) {
  return update.status === 'downloading' || update.status === 'ready'
}

export const useAppUpdateStore = create<AppUpdateState>()((set, get) => ({
  update: { status: 'idle' },
  currentVersion: null,
  check: async (manual = false) => {
    if (!APP_UPDATES_ENABLED || busy(get().update)) return
    if (checking) {
      if (manual) set({ update: { status: 'checking' } })
      return checking
    }
    clearTimeout(upToDateTimer)
    if (manual) set({ update: { status: 'checking' } })
    checking = (async () => {
      try {
        const [found, currentVersion] = await Promise.all([
          check(),
          getVersion()
        ])
        lastCheckedAt = Date.now()
        if (busy(get().update)) return
        const previous = pending
        pending = found
        if (previous && previous !== found) void previous.close()
        if (found) {
          set({
            currentVersion,
            update: { status: 'available', version: found.version }
          })
        } else if (get().update.status === 'checking') {
          set({
            currentVersion,
            update: { status: 'upToDate', version: currentVersion }
          })
          upToDateTimer = setTimeout(() => {
            if (get().update.status === 'upToDate')
              set({ update: { status: 'idle' } })
          }, UP_TO_DATE_FOR)
        } else {
          set({ currentVersion, update: { status: 'idle' } })
        }
      } catch (error) {
        console.warn('hoshi: failed to check for an app update', error)
        if (get().update.status === 'checking')
          set({ update: { status: 'checkFailed', message: String(error) } })
      } finally {
        checking = null
      }
    })()
    return checking
  },
  download: async () => {
    const found = pending
    if (!found || busy(get().update)) return
    const { version } = found
    set({ update: { status: 'downloading', version, percent: null } })
    let total = 0
    let received = 0
    try {
      await found.download((event) => {
        if (event.event === 'Started') total = event.data.contentLength ?? 0
        if (event.event === 'Progress') {
          received += event.data.chunkLength
          set({
            update: {
              status: 'downloading',
              version,
              percent: downloadPercent(received, total)
            }
          })
        }
      })
      set({ update: { status: 'ready', version } })
    } catch (error) {
      console.error('hoshi: failed to download the app update', error)
      set({ update: { status: 'failed', version } })
    }
  },
  restart: async () => {
    const found = pending
    const { update } = get()
    if (!found || update.status !== 'ready') return
    try {
      await found.install()
      await relaunch()
    } catch (error) {
      console.error('hoshi: failed to install the app update', error)
      set({ update: { status: 'failed', version: update.version } })
    }
  }
}))

/** Checks now, every half hour, and when the window comes back after a while. */
export function watchAppUpdates() {
  const run = () => void useAppUpdateStore.getState().check()
  const onFocus = () => {
    if (checkDue(lastCheckedAt, Date.now())) run()
  }
  run()
  const timer = setInterval(run, CHECK_EVERY)
  window.addEventListener('focus', onFocus)
  return () => {
    clearInterval(timer)
    window.removeEventListener('focus', onFocus)
  }
}
