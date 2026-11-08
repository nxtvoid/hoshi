import { useEffect } from 'react'
import { m } from '@/paraglide/messages'
import { useAppStore } from './app-store'
import { useSettingsStore } from './settings-store'
import { type ThreadNotice, threadNotices } from './thread-signals'
import {
  isPermissionGranted,
  requestPermission,
  sendNotification
} from '@tauri-apps/plugin-notification'

let permission: Promise<boolean> | null = null

function allowed(): Promise<boolean> {
  permission ??= isPermissionGranted()
    .then(
      (granted) =>
        granted || requestPermission().then((answer) => answer === 'granted')
    )
    .catch(() => false)
  return permission
}

const BODY: Record<ThreadNotice['kind'], (project: string) => string> = {
  finished: (project) => m.notification_finished({ project }),
  failed: (project) => m.notification_failed({ project }),
  waiting: (project) => m.notification_waiting({ project })
}

/**
 * A system notification for each thread that finishes, fails or asks for input while the
 * window is in the background, unless turned off in the settings.
 */
export function useThreadNotifications() {
  useEffect(
    () =>
      useAppStore.subscribe((state, previous) => {
        if (state.threads === previous.threads || !previous.ready) return
        if (!useSettingsStore.getState().settings?.general.notifications) return
        if (document.hasFocus()) return
        const notices = threadNotices(previous.threads, state.threads)
        if (notices.length === 0) return
        void allowed().then((granted) => {
          if (!granted) return
          for (const { threadId, kind } of notices) {
            const thread = state.threads.find(({ id }) => id === threadId)
            const project = state.projects.find(
              ({ id }) => id === thread?.projectId
            )
            sendNotification({
              title: thread?.title || m.untitled_thread(),
              body: BODY[kind](project?.title ?? m.chat())
            })
          }
        })
      }),
    []
  )
}
