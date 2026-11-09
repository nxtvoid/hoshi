import { create } from 'zustand'
import type { IconSvgElement } from '@hugeicons/react'

/**
 * `destructive` for what can't be undone, `warning` to tell about a refusal, `success` for
 * something that went through.
 */
export type DialogTone = 'default' | 'destructive' | 'warning' | 'success'

export type DialogRequest = {
  title: string
  description?: string
  /** The button that goes ahead. */
  confirmLabel: string
  /** A second way to go ahead, shown before the first. */
  alternativeLabel?: string
  /** The second way loses something, as reverting files does. */
  alternativeDestructive?: boolean
  /** `false` leaves only the confirm button, for a notice. */
  cancelLabel?: string | false
  tone?: DialogTone
  /** Says what going ahead does, in place of the tone's own icon. */
  icon?: IconSvgElement
}

/** Which button the user pressed; `null` when they cancelled. */
export type DialogAnswer = 'confirm' | 'alternative' | null

type Pending = DialogRequest & { resolve: (answer: DialogAnswer) => void }

type DialogState = {
  /** The one on screen. It stays while `open` turns false, so it can animate out. */
  current: Pending | null
  open: boolean
  queue: Pending[]
  settle: (answer: DialogAnswer) => void
}

export const useDialogStore = create<DialogState>()((set, get) => ({
  current: null,
  open: false,
  queue: [],
  settle: (answer) => {
    const { current, open, queue } = get()
    if (!current || !open) return
    current.resolve(answer)
    const [next, ...rest] = queue
    set(next ? { current: next, open: true, queue: rest } : { open: false })
  }
}))

function show(request: DialogRequest): Promise<DialogAnswer> {
  return new Promise((resolve) => {
    const pending = { ...request, resolve }
    const { open, queue } = useDialogStore.getState()
    useDialogStore.setState(
      open ? { queue: [...queue, pending] } : { current: pending, open: true }
    )
  })
}

/** Asks the user in the app's own dialog; resolves to whether they went ahead. */
export async function confirm(request: DialogRequest): Promise<boolean> {
  return (await show(request)) === 'confirm'
}

/** Asks the user to pick one of two ways to go ahead, or to cancel. */
export function choose(
  request: DialogRequest & { alternativeLabel: string }
): Promise<DialogAnswer> {
  return show(request)
}

/** Tells the user something that needs no answer; resolves once they close it. */
export async function notify(
  request: Omit<DialogRequest, 'cancelLabel'>
): Promise<void> {
  await show({ ...request, cancelLabel: false })
}
