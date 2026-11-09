export type EditAction = 'cut' | 'copy' | 'paste' | 'selectAll'

/** Where a right-click landed: text the user can edit, the terminal, or anything else. */
export type EditTarget = 'text' | 'terminal' | 'readOnly'

export type EditMenu = Record<EditAction, boolean>

export const EDIT_ACTIONS: EditAction[] = ['cut', 'copy', 'paste', 'selectAll']

/** Which items the edit menu enables; `null` when it has nothing to offer there. */
export function editMenu(
  target: EditTarget,
  selected: boolean
): EditMenu | null {
  if (target === 'readOnly' && !selected) return null
  return {
    cut: target === 'text' && selected,
    copy: selected,
    paste: target !== 'readOnly',
    selectAll: target !== 'readOnly'
  }
}

const SHORTCUT_KEYS: Record<EditAction, string> = {
  cut: 'X',
  copy: 'C',
  paste: 'V',
  selectAll: 'A'
}

export function editShortcut(action: EditAction, mac: boolean) {
  return mac ? `⌘${SHORTCUT_KEYS[action]}` : `Ctrl ${SHORTCUT_KEYS[action]}`
}

const TEXT_INPUT_TYPES = new Set([
  'text',
  'search',
  'url',
  'tel',
  'email',
  'password',
  'number'
])

/** Input types that hold text the user types. */
export function isTextInputType(type: string) {
  return TEXT_INPUT_TYPES.has(type.toLowerCase())
}
