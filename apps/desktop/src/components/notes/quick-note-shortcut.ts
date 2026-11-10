import { isMac } from '@/components/brand/EditorIcon'

export const QUICK_NOTE_SHORTCUT = () => (isMac() ? '⇧⌘N' : 'Ctrl Shift N')

/** Whether `event` is the shortcut that opens a quick note, from anywhere in the app. */
export function isQuickNoteShortcut(event: KeyboardEvent) {
  const modifier = isMac() ? event.metaKey : event.ctrlKey
  return modifier && event.shiftKey && !event.altKey && event.code === 'KeyN'
}
