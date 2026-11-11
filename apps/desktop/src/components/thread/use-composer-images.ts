import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { prepareImage } from '@/lib/images'
import { useImageStore } from '@/state/image-store'
import type { MessageAttachment } from '@/bindings/MessageAttachment'
import {
  type DraftImage,
  type SavedImage,
  useDraftStore
} from '@/state/draft-store'

export type ComposerImage = {
  key: string
  name: string
  /** In bytes, as added. */
  size: number
  /** Shown while the image is saved, and after; empty until a resumed one is read. */
  previewUrl: string
  /** Set once the engine saved it. */
  saved: MessageAttachment | null
  failed: boolean
  /** Shown as a chip in the text; without its chip, it is left out. */
  chipped: boolean
}

/** Removes a saved image that no message will carry. */
function discardSaved(saved: MessageAttachment) {
  if (saved.type === 'image') useImageStore.getState().forget(saved.id)
  void invoke('discard_image', { image: saved })
}

const isSavedImage = (saved: MessageAttachment | null): saved is SavedImage =>
  saved?.type === 'image'

/** The images to keep in the draft: those the engine saved. */
function draftImages(images: ComposerImage[]): DraftImage[] {
  return images.flatMap((image) =>
    isSavedImage(image.saved)
      ? [{ key: image.key, image: image.saved, chipped: image.chipped }]
      : []
  )
}

/** The images the draft kept when its composer last closed. */
function resumed(draftKey: string): ComposerImage[] {
  const { urls } = useImageStore.getState()
  return (useDraftStore.getState().images[draftKey] ?? []).map(
    ({ key, image, chipped }) => ({
      key,
      name: image.name,
      size: Number(image.size),
      previewUrl: urls[image.id] ?? '',
      saved: image,
      failed: false,
      chipped
    })
  )
}

/** Adds an image saved after its composer closed to the draft it was added to. */
function keepInDraft(draftKey: string, image: ComposerImage) {
  if (!isSavedImage(image.saved)) return
  const store = useDraftStore.getState()
  const kept = store.images[draftKey] ?? []
  if (kept.some((draft) => draft.key === image.key)) return
  store.saveImages(draftKey, [
    ...kept,
    { key: image.key, image: image.saved, chipped: image.chipped }
  ])
}

/**
 * Images added to the draft, saved by the engine as soon as they are added. They stay with
 * the draft when the composer closes, and come back when it opens again.
 */
export function useComposerImages(draftKey: string) {
  const [images, setImages] = useState(() => resumed(draftKey))
  const imagesRef = useRef(images)
  useLayoutEffect(() => {
    imagesRef.current = images
  }, [images])
  const mounted = useRef(true)

  const update = (key: string, change: Partial<ComposerImage>) =>
    setImages((current) =>
      current.map((image) =>
        image.key === key ? { ...image, ...change } : image
      )
    )

  /** Returns the images as added, before they are saved. */
  const add = (files: File[]): ComposerImage[] =>
    files.map((file) => {
      const key = crypto.randomUUID()
      const image: ComposerImage = {
        key,
        name: file.name || 'image.png',
        size: file.size,
        previewUrl: URL.createObjectURL(file),
        saved: null,
        failed: false,
        chipped: false
      }
      setImages((current) => [...current, image])
      prepareImage(file)
        .then(async (prepared) => {
          const saved = await invoke<MessageAttachment>('save_image', {
            name: prepared.name || image.name,
            mimeType: prepared.mimeType,
            data: prepared.data
          })
          if (saved.type === 'image') {
            useImageStore
              .getState()
              .keep(
                saved.id,
                `data:${prepared.mimeType};base64,${prepared.data}`
              )
          }
          const current = imagesRef.current.find(
            (candidate) => candidate.key === key
          )
          if (!current) discardSaved(saved)
          else if (mounted.current) update(key, { saved })
          else keepInDraft(draftKey, { ...current, saved })
        })
        .catch((error) => {
          console.error('hoshi: failed to attach an image', error)
          update(key, { failed: true })
        })
      return image
    })

  /** These images show as chips in the text. */
  const markChipped = (keys: string[]) => {
    imagesRef.current = imagesRef.current.map((image) =>
      keys.includes(image.key) ? { ...image, chipped: true } : image
    )
    setImages((current) =>
      current.map((image) =>
        keys.includes(image.key) ? { ...image, chipped: true } : image
      )
    )
  }

  /** Puts back images a message was sent with, already saved; `urls` shows them. */
  const restore = (
    attachments: MessageAttachment[],
    urls: Record<string, string | null>
  ) => {
    const restored = attachments.filter(isSavedImage).map((attachment) => ({
      key: crypto.randomUUID(),
      name: attachment.name,
      size: Number(attachment.size),
      previewUrl: urls[attachment.id] ?? '',
      saved: attachment,
      failed: false,
      chipped: false
    }))
    setImages((current) => [...current, ...restored])
  }

  const remove = (key: string) => {
    const image = imagesRef.current.find((candidate) => candidate.key === key)
    if (!image) return
    URL.revokeObjectURL(image.previewUrl)
    if (image.saved) discardSaved(image.saved)
    imagesRef.current = imagesRef.current.filter(
      (candidate) => candidate.key !== key
    )
    setImages((current) => current.filter((candidate) => candidate.key !== key))
  }

  /** Takes the images out of the draft to send them; their files and previews stay. */
  const take = () => {
    const taken = imagesRef.current
    imagesRef.current = []
    setImages([])
    return taken
  }

  /** Puts back images taken for a send that failed, ahead of any added since. */
  const putBack = (taken: ComposerImage[]) => {
    if (mounted.current) {
      setImages((current) => [...taken, ...current])
      return
    }
    for (const image of taken) keepInDraft(draftKey, image)
  }

  /** After sending: the files stay, the transcript shows them now. */
  const release = (taken: ComposerImage[]) => {
    for (const image of taken) URL.revokeObjectURL(image.previewUrl)
  }

  /** After sending: images taken that the message left out, whose files go. */
  const discard = (taken: ComposerImage[]) => {
    for (const image of taken) {
      URL.revokeObjectURL(image.previewUrl)
      if (image.saved) discardSaved(image.saved)
    }
  }

  useEffect(() => {
    useDraftStore.getState().saveImages(draftKey, draftImages(images))
  }, [images, draftKey])

  useEffect(() => {
    for (const image of imagesRef.current) {
      if (image.previewUrl || !isSavedImage(image.saved)) continue
      const { id, mimeType } = image.saved
      void useImageStore
        .getState()
        .load(id, mimeType)
        .then(() => {
          const url = useImageStore.getState().urls[id]
          if (url) update(image.key, { previewUrl: url })
        })
    }
  }, [])

  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
      for (const image of imagesRef.current) {
        URL.revokeObjectURL(image.previewUrl)
      }
    }
  }, [])

  return {
    images,
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
