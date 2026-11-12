import { invoke } from '@tauri-apps/api/core'
import { openUrl } from '@tauri-apps/plugin-opener'
import { useSettingsStore } from '@/state/settings-store'

/**
 * Opens a link from the chat where the settings say: hoshi's browser, in the shown thread's
 * side panel, or the default browser, which `external` (Ctrl or ⌘ held) always picks.
 */
export async function openWebLink(href: string, external: boolean) {
  const inHoshi =
    useSettingsStore.getState().settings?.browser.openLinksIn === 'hoshi'
  if (external || !inHoshi || !/^https?:\/\//i.test(href)) {
    return openUrl(href)
  }
  // Loaded on click: the stores start syncing with the app as they load.
  const [{ useAppStore }, { usePanelStore }] = await Promise.all([
    import('@/state/app-store'),
    import('@/state/panel-store')
  ])
  const threadId = useAppStore.getState().selectedThreadId
  if (!threadId) return openUrl(href)
  usePanelStore.getState().openSurface(threadId, 'browser')
  await invoke('browser_open', { threadId, url: href }).catch(() =>
    openUrl(href)
  )
}
