import { invoke } from '@tauri-apps/api/core'
import type { Editor } from '@/bindings/Editor'
import type { MessageAttachment } from '@/bindings/MessageAttachment'
import { opensInEditor } from '@/lib/local-paths'
import { useEditorStore } from '@/state/editor-store'
import { useDetailsStore } from '@/state/details-store'

/** The code editor the user opens folders in, if one is installed. */
function codeEditor(): Editor | null {
  const preferred = useEditorStore.getState().preferred
  if (preferred === 'fileManager') return null
  const editors = (useDetailsStore.getState().editors ?? []).filter(
    (editor) => editor !== 'fileManager'
  )
  return preferred && editors.includes(preferred)
    ? preferred
    : (editors[0] ?? null)
}

/** Opens a file an agent's answer names: code in the user's editor, the rest in its app. */
export function openLocalFile(path: string) {
  const editor = opensInEditor(path) ? codeEditor() : null
  invoke('open_local_file', { path, editor }).catch((error) =>
    console.error('hoshi: failed to open a file', error)
  )
}

/** Opens an image or file a message was sent with, where hoshi saved it. */
export function openAttachment(attachment: MessageAttachment) {
  invoke<string | null>('get_attachment_path', { attachment })
    .then((path) => {
      if (path) openLocalFile(path)
    })
    .catch((error) => console.error('hoshi: failed to open a file', error))
}
