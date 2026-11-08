import { create } from 'zustand'
import { relaunch } from '@tauri-apps/plugin-process'
import { getVersion } from '@tauri-apps/api/app'
import { type AppUpdate, downloadPercent } from '@/lib/updates'
import { check, type Update } from '@tauri-apps/plugin-updater'

const CHECK_EVERY = 6 * 60 * 60 * 1000

type AppUpdateState = {
  update: AppUpdate
  /** The running build's version; `null` until read. */
  currentVersion: string | null
  /** Reads the release feed; dev builds never update. */
  check: () => Promise<void>
  /** Downloads and installs the update, then restarts into it. */
  install: () => Promise<void>
}

/** The plugin's handle on the release found last; it can't live in the store. */
let pending: Update | null = null

export const useAppUpdateStore = create<AppUpdateState>()((set, get) => ({
  update: { status: 'idle' },
  currentVersion: null,
  check: async () => {
    if (!import.meta.env.PROD || get().update.status === 'downloading') return
    try {
      const [found, currentVersion] = await Promise.all([check(), getVersion()])
      pending = found
      set({
        currentVersion,
        update: found
          ? { status: 'available', version: found.version }
          : { status: 'idle' }
      })
    } catch (error) {
      console.warn('hoshi: failed to check for an app update', error)
    }
  },
  install: async () => {
    const found = pending
    if (!found || get().update.status === 'downloading') return
    const { version } = found
    set({ update: { status: 'downloading', version, percent: null } })
    let total = 0
    let received = 0
    try {
      await found.downloadAndInstall((event) => {
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
      await relaunch()
    } catch (error) {
      console.error('hoshi: failed to install the app update', error)
      set({ update: { status: 'failed', version } })
    }
  }
}))

/** Checks now and every few hours after. */
export function watchAppUpdates() {
  const run = () => void useAppUpdateStore.getState().check()
  run()
  const timer = setInterval(run, CHECK_EVERY)
  return () => clearInterval(timer)
}
