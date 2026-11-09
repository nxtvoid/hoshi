import { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { prepareImage } from '@/lib/images'
import type { MessageAttachment } from '@/bindings/MessageAttachment'

export type ComposerImage = {
  key: string
  name: string
  /** In bytes, as added. */
  size: number
  /** Shown while the image is saved, and after. */
  previewUrl: string
  /** Set once the engine saved it. */
  saved: MessageAttachment | null
  failed: boolean
}

type SavedImage = Extract<MessageAttachment, { type: 'image' }>

/** Images added to the draft, saved by the engine as soon as they are added. */
export function useComposerImages() {
  const [images, setImages] = useState<ComposerImage[]>([])
  const imagesRef = useRef(images)
  imagesRef.current = images
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
        failed: false
      }
      setImages((current) => [...current, image])
      prepareImage(file)
        .then((prepared) =>
          invoke<MessageAttachment>('save_image', {
            name: prepared.name || image.name,
            mimeType: prepared.mimeType,
            data: prepared.data
          })
        )
        .then((saved) => {
          const kept = imagesRef.current.some(
            (candidate) => candidate.key === key
          )
          if (kept) update(key, { saved })
          else void invoke('discard_image', { image: saved })
        })
        .catch((error) => {
          console.error('hoshi: failed to attach an image', error)
          update(key, { failed: true })
        })
      return image
    })

  /** Puts back images a message was sent with, already saved; `urls` shows them. */
  const restore = (
    attachments: MessageAttachment[],
    urls: Record<string, string | null>
  ) => {
    const restored = attachments
      .filter(
        (attachment): attachment is SavedImage => attachment.type === 'image'
      )
      .map((attachment) => ({
        key: crypto.randomUUID(),
        name: attachment.name,
        size: Number(attachment.size),
        previewUrl: urls[attachment.id] ?? '',
        saved: attachment,
        failed: false
      }))
    setImages((current) => [...current, ...restored])
  }

  const remove = (key: string) => {
    const image = imagesRef.current.find((candidate) => candidate.key === key)
    if (!image) return
    URL.revokeObjectURL(image.previewUrl)
    if (image.saved) void invoke('discard_image', { image: image.saved })
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
    for (const image of taken) {
      URL.revokeObjectURL(image.previewUrl)
      if (image.saved) void invoke('discard_image', { image: image.saved })
    }
  }

  /** After sending: the files stay, the transcript shows them now. */
  const release = (taken: ComposerImage[]) => {
    for (const image of taken) URL.revokeObjectURL(image.previewUrl)
  }

  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
      for (const image of imagesRef.current) {
        URL.revokeObjectURL(image.previewUrl)
        if (image.saved) void invoke('discard_image', { image: image.saved })
      }
    }
  }, [])

  return {
    images,
    add,
    restore,
    remove,
    take,
    putBack,
    release,
    saving: images.some((image) => !image.saved && !image.failed),
    saved: images.flatMap((image) => (image.saved ? [image.saved] : []))
  }
}
