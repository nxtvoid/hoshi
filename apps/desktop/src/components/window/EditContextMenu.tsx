import { m } from '@/paraglide/messages'
import type { Terminal } from '@xterm/xterm'
import { invoke } from '@tauri-apps/api/core'
import { terminalAt } from './edit-terminals'
import { saveNote, useNotesStore } from '@/state/notes-store'
import { useAppStore } from '@/state/app-store'
import { scopeProjectId } from '@/state/scope'
import { useSidebarStore } from '@/state/sidebar-store'
import { useEffect, useMemo, useState } from 'react'
import { isMac } from '@/components/brand/EditorIcon'
import {
  EDIT_ACTIONS,
  type EditAction,
  type EditMenu,
  editMenu,
  editShortcut,
  isTextInputType
} from '@/lib/edit-menu'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuShortcut
} from '@hoshi/ui/components/dropdown-menu'

const LABELS: Record<EditAction, () => string> = {
  cut: m.edit_cut,
  copy: m.edit_copy,
  paste: m.edit_paste,
  selectAll: m.edit_select_all
}

type Opened = {
  x: number
  y: number
  menu: EditMenu
  run: Record<EditAction, () => void>
  /** Where focus goes back when the menu closes from the keyboard. */
  focus: HTMLElement | null
  /** The selected text, to keep as a note; empty when none is, or it is a password. */
  selection: string
}

/** hoshi's right-click menu over text, in place of the webview's; dev builds keep the webview's. */
export function EditContextMenu() {
  const [opened, setOpened] = useState<Opened | null>(null)
  const [open, setOpen] = useState(false)

  useEffect(() => {
    if (import.meta.env.DEV) return
    const onContextMenu = (event: MouseEvent) => {
      if (event.defaultPrevented) return
      event.preventDefault()
      const found =
        event.target instanceof Element ? editTarget(event.target) : null
      if (!found) {
        setOpen(false)
        return
      }
      setOpened({ ...found, x: event.clientX, y: event.clientY })
      setOpen(true)
    }
    window.addEventListener('contextmenu', onContextMenu)
    return () => window.removeEventListener('contextmenu', onContextMenu)
  }, [])

  useEffect(() => {
    if (!open) return
    const close = () => setOpen(false)
    window.addEventListener('scroll', close, true)
    window.addEventListener('blur', close)
    window.addEventListener('resize', close)
    return () => {
      window.removeEventListener('scroll', close, true)
      window.removeEventListener('blur', close)
      window.removeEventListener('resize', close)
    }
  }, [open])

  const anchor = useMemo(() => {
    if (!opened) return null
    const { x, y } = opened
    return {
      getBoundingClientRect: () =>
        DOMRect.fromRect({ x, y, width: 0, height: 0 })
    }
  }, [opened])

  if (!opened) return null

  return (
    <DropdownMenu open={open} onOpenChange={setOpen} modal={false}>
      <DropdownMenuContent
        anchor={anchor}
        side='bottom'
        align='start'
        finalFocus={(closeType) =>
          closeType === 'keyboard' ? (opened.focus ?? false) : false
        }
        className='min-w-44'
      >
        {EDIT_ACTIONS.map((action) => (
          <DropdownMenuItem
            key={action}
            disabled={!opened.menu[action]}
            onClick={opened.run[action]}
          >
            {LABELS[action]()}
            <DropdownMenuShortcut className='tracking-normal'>
              {editShortcut(action, isMac())}
            </DropdownMenuShortcut>
          </DropdownMenuItem>
        ))}
        {opened.selection ? (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem
              onClick={() => void saveToNotes(opened.selection)}
            >
              {m.edit_save_to_notes()}
            </DropdownMenuItem>
          </>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

/** Keeps the selection as a note of the open thread's project, or a general one. */
async function saveToNotes(text: string) {
  const state = useAppStore.getState()
  const thread = state.threads.find(
    (candidate) => candidate.id === state.selectedThreadId
  )
  const projectId = thread
    ? thread.projectId
    : scopeProjectId(state, useSidebarStore.getState().view)
  try {
    await saveNote(crypto.randomUUID(), projectId, text)
    useNotesStore.getState().captured()
  } catch (error) {
    console.error('hoshi: failed to save a note', error)
  }
}

function editTarget(element: Element): Omit<Opened, 'x' | 'y'> | null {
  const terminal = terminalAt(element)
  if (terminal) {
    const menu = editMenu('terminal', terminal.hasSelection())
    return (
      menu && {
        menu,
        run: terminalActions(terminal),
        focus: terminal.textarea ?? null,
        selection: terminal.getSelection().trim()
      }
    )
  }
  const field = element.closest('input, textarea')
  if (
    (field instanceof HTMLInputElement && isTextInputType(field.type)) ||
    field instanceof HTMLTextAreaElement
  ) {
    const editable = !field.readOnly && !field.disabled
    const selected =
      field.selectionStart !== null &&
      field.selectionStart !== field.selectionEnd
    const menu = editMenu(editable ? 'text' : 'readOnly', selected)
    const secret =
      field instanceof HTMLInputElement && field.type === 'password'
    const selection = secret
      ? ''
      : field.value
          .slice(field.selectionStart ?? 0, field.selectionEnd ?? 0)
          .trim()
    return menu && { menu, run: textActions(field), focus: field, selection }
  }
  const host = element.closest(
    '[contenteditable]:not([contenteditable="false"])'
  )
  const editable = host instanceof HTMLElement && host.isContentEditable
  const selection = window.getSelection()
  const selected =
    selection !== null &&
    !selection.isCollapsed &&
    selection.toString().trim() !== ''
  const menu = editMenu(editable ? 'text' : 'readOnly', selected)
  if (!menu) return null
  const focus = editable ? host : null
  return {
    menu,
    run: textActions(focus),
    focus,
    selection: selected ? (selection?.toString().trim() ?? '') : ''
  }
}

/** Puts focus and the selection back where the right-click found them. */
function selectionRestorer(element: HTMLElement | null) {
  if (
    element instanceof HTMLInputElement ||
    element instanceof HTMLTextAreaElement
  ) {
    const { selectionStart, selectionEnd, selectionDirection } = element
    return () => {
      element.focus({ preventScroll: true })
      if (selectionStart !== null && selectionEnd !== null) {
        element.setSelectionRange(
          selectionStart,
          selectionEnd,
          selectionDirection ?? undefined
        )
      }
    }
  }
  const selection = window.getSelection()
  const range =
    selection && selection.rangeCount > 0
      ? selection.getRangeAt(0).cloneRange()
      : null
  return () => {
    element?.focus({ preventScroll: true })
    if (!range) return
    const current = window.getSelection()
    current?.removeAllRanges()
    current?.addRange(range)
  }
}

function textActions(
  element: HTMLElement | null
): Record<EditAction, () => void> {
  const restore = selectionRestorer(element)
  const exec = (command: string) => () => {
    restore()
    document.execCommand(command)
  }
  return {
    cut: exec('cut'),
    copy: exec('copy'),
    selectAll: exec('selectAll'),
    paste: () => {
      void readClipboardText().then((text) => {
        if (!text) return
        restore()
        document.execCommand('insertText', false, text)
      })
    }
  }
}

function terminalActions(terminal: Terminal): Record<EditAction, () => void> {
  return {
    cut: () => {},
    copy: () => {
      const text = terminal.getSelection()
      if (text) void navigator.clipboard.writeText(text).catch(() => {})
      terminal.focus()
    },
    paste: () => {
      void readClipboardText().then((text) => {
        terminal.focus()
        if (text) terminal.paste(text)
      })
    },
    selectAll: () => terminal.selectAll()
  }
}

function readClipboardText() {
  return invoke<string>('read_clipboard_text').catch(() => '')
}
