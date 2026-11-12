import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { m } from '@/paraglide/messages'
import { prepareImage } from '@/lib/images'
import { toastManager } from '@hoshi/ui/components/toast'
import { useImageStore } from '@/state/image-store'
import type { MessageAttachment } from '@/bindings/MessageAttachment'
import {
  attachmentMimeType,
  classifyAttachment,
  withAttachmentType
} from '@/lib/attachment-files'
import {
  type DraftAttachment,
  type SavedAttachment,
  useDraftStore
} from '@/state/draft-store'

export type ComposerAttachment = {
  key: string
  /** An image the agent sees, or a file it reads from disk. */
  kind: 'image' | 'file'
  name: string
  /** In bytes, as added. */
  size: number
  /** An image's preview, shown while it is saved and after; empty for a file, and until a
   * resumed image is read. */
  previewUrl: string
  /** Set once the engine saved it. */
  saved: SavedAttachment | null
  failed: boolean
  /** Shown as a chip in the text; without its chip, it is left out. */
  chipped: boolean
}

/** Removes a saved image or file that no message will carry. */
function discardSaved(saved: SavedAttachment) {
  if (saved.type === 'image') useImageStore.getState().forget(saved.id)
  void invoke('discard_attachment', { attachment: saved })
}

const isSaved = (
  attachment: MessageAttachment | null
): attachment is SavedAttachment =>
  attachment?.type === 'image' || attachment?.type === 'file'

/** The attachments to keep in the draft: those the engine saved. */
function draftAttachments(
  attachments: ComposerAttachment[]
): DraftAttachment[] {
  return attachments.flatMap((attachment) =>
    attachment.saved
      ? [
          {
            key: attachment.key,
            attachment: attachment.saved,
            chipped: attachment.chipped
          }
        ]
      : []
  )
}

/** A saved attachment as the composer shows it, its preview from `urls`. */
function fromSaved(
  key: string,
  saved: SavedAttachment,
  chipped: boolean,
  urls: Record<string, string | null>
): ComposerAttachment {
  return {
    key,
    kind: saved.type,
    name: saved.name,
    size: Number(saved.size),
    previewUrl: saved.type === 'image' ? (urls[saved.id] ?? '') : '',
    saved,
    failed: false,
    chipped
  }
}

/** The attachments the draft kept when its composer last closed. */
function resumed(draftKey: string): ComposerAttachment[] {
  const { urls } = useImageStore.getState()
  return (useDraftStore.getState().attachments[draftKey] ?? []).map(
    ({ key, attachment, chipped }) => fromSaved(key, attachment, chipped, urls)
  )
}

/** Adds an attachment saved after its composer closed to the draft it was added to. */
function keepInDraft(draftKey: string, attachment: ComposerAttachment) {
  if (!attachment.saved) return
  const store = useDraftStore.getState()
  const kept = store.attachments[draftKey] ?? []
  if (kept.some((draft) => draft.key === attachment.key)) return
  store.saveAttachments(draftKey, [
    ...kept,
    {
      key: attachment.key,
      attachment: attachment.saved,
      chipped: attachment.chipped
    }
  ])
}

/** Saves an image as a provider takes it, keeping its preview for the transcript. */
async function saveImage(file: File, name: string): Promise<SavedAttachment> {
  const prepared = await prepareImage(withAttachmentType(file))
  const saved = await invoke<MessageAttachment>('save_image', {
    name: prepared.name || name,
    mimeType: prepared.mimeType,
    data: prepared.data
  })
  if (saved.type !== 'image') throw new Error('not saved as an image')
  useImageStore
    .getState()
    .keep(saved.id, `data:${prepared.mimeType};base64,${prepared.data}`)
  return saved
}

/** Saves a file as it is: its bytes go as the request's body, not as JSON. */
async function saveFile(file: File, name: string): Promise<SavedAttachment> {
  const saved = await invoke<MessageAttachment>(
    'save_file',
    await file.arrayBuffer(),
    {
      headers: {
        'x-name': encodeURIComponent(name),
        'x-mime-type': encodeURIComponent(attachmentMimeType(file))
      }
    }
  )
  if (saved.type !== 'file') throw new Error('not saved as a file')
  return saved
}

/**
 * Images and files added to the draft, saved by the engine as soon as they are added. They
 * stay with the draft when the composer closes, and come back when it opens again.
 */
export function useComposerAttachments(draftKey: string) {
  const [attachments, setAttachments] = useState(() => resumed(draftKey))
  const attachmentsRef = useRef(attachments)
  useLayoutEffect(() => {
    attachmentsRef.current = attachments
  }, [attachments])
  const mounted = useRef(true)

  const update = (key: string, change: Partial<ComposerAttachment>) =>
    setAttachments((current) =>
      current.map((attachment) =>
        attachment.key === key ? { ...attachment, ...change } : attachment
      )
    )

  /** Returns the attachments as added, before they are saved. */
  const add = (files: File[]): ComposerAttachment[] =>
    files.map((file) => {
      const key = crypto.randomUUID()
      const kind = classifyAttachment(file) === 'image' ? 'image' : 'file'
      const name = file.name || (kind === 'image' ? 'image.png' : 'file')
      const attachment: ComposerAttachment = {
        key,
        kind,
        name,
        size: file.size,
        previewUrl: kind === 'image' ? URL.createObjectURL(file) : '',
        saved: null,
        failed: false,
        chipped: false
      }
      setAttachments((current) => [...current, attachment])
      const saving =
        kind === 'image' ? saveImage(file, name) : saveFile(file, name)
      saving
        .then((saved) => {
          const current = attachmentsRef.current.find(
            (candidate) => candidate.key === key
          )
          if (!current) discardSaved(saved)
          else if (mounted.current) update(key, { saved })
          else keepInDraft(draftKey, { ...current, saved })
        })
        .catch((error) => {
          console.error('hoshi: failed to attach a file', error)
          update(key, { failed: true })
          toastManager.add({
            type: 'error',
            title: m.composer_attachment_failed({ name }),
            description: error instanceof Error ? error.message : String(error)
          })
        })
      return attachment
    })

  /** These attachments show as chips in the text. */
  const markChipped = (keys: string[]) => {
    attachmentsRef.current = attachmentsRef.current.map((attachment) =>
      keys.includes(attachment.key)
        ? { ...attachment, chipped: true }
        : attachment
    )
    setAttachments((current) =>
      current.map((attachment) =>
        keys.includes(attachment.key)
          ? { ...attachment, chipped: true }
          : attachment
      )
    )
  }

  /** Puts back images and files a message was sent with, already saved; `urls` shows them. */
  const restore = (
    sent: MessageAttachment[],
    urls: Record<string, string | null>
  ) => {
    const restored = sent
      .filter(isSaved)
      .map((attachment) =>
        fromSaved(crypto.randomUUID(), attachment, false, urls)
      )
    setAttachments((current) => [...current, ...restored])
  }

  const remove = (key: string) => {
    const attachment = attachmentsRef.current.find(
      (candidate) => candidate.key === key
    )
    if (!attachment) return
    if (attachment.previewUrl) URL.revokeObjectURL(attachment.previewUrl)
    if (attachment.saved) discardSaved(attachment.saved)
    attachmentsRef.current = attachmentsRef.current.filter(
      (candidate) => candidate.key !== key
    )
    setAttachments((current) =>
      current.filter((candidate) => candidate.key !== key)
    )
  }

  /** Takes the attachments out of the draft to send them; their files and previews stay. */
  const take = () => {
    const taken = attachmentsRef.current
    attachmentsRef.current = []
    setAttachments([])
    return taken
  }

  /** Puts back attachments taken for a send that failed, ahead of any added since. */
  const putBack = (taken: ComposerAttachment[]) => {
    if (mounted.current) {
      setAttachments((current) => [...taken, ...current])
      return
    }
    for (const attachment of taken) keepInDraft(draftKey, attachment)
  }

  /** After sending: the files stay, the transcript shows them now. */
  const release = (taken: ComposerAttachment[]) => {
    for (const attachment of taken) {
      if (attachment.previewUrl) URL.revokeObjectURL(attachment.previewUrl)
    }
  }

  /** After sending: attachments taken that the message left out, whose files go. */
  const discard = (taken: ComposerAttachment[]) => {
    for (const attachment of taken) {
      if (attachment.previewUrl) URL.revokeObjectURL(attachment.previewUrl)
      if (attachment.saved) discardSaved(attachment.saved)
    }
  }

  useEffect(() => {
    useDraftStore
      .getState()
      .saveAttachments(draftKey, draftAttachments(attachments))
  }, [attachments, draftKey])

  useEffect(() => {
    for (const attachment of attachmentsRef.current) {
      const { saved } = attachment
      if (attachment.previewUrl || saved?.type !== 'image') continue
      void useImageStore
        .getState()
        .load(saved.id, saved.mimeType)
        .then(() => {
          const url = useImageStore.getState().urls[saved.id]
          if (url) update(attachment.key, { previewUrl: url })
        })
    }
  }, [])

  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
      for (const attachment of attachmentsRef.current) {
        if (attachment.previewUrl) URL.revokeObjectURL(attachment.previewUrl)
      }
    }
  }, [])

  return {
    attachments,
    add,
    markChipped,
    restore,
    remove,
    take,
    putBack,
    release,
    discard
  }
}
