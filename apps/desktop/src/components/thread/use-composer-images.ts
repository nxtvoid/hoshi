import { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { prepareImage } from '@/lib/images'
import type { MessageAttachment } from '@/bindings/MessageAttachment'

export type ComposerImage = {
  key: string
  name: string
  /** Shown while the image is saved, and after. */
  previewUrl: string
  /** Set once the engine saved it. */
  saved: MessageAttachment | null
  failed: boolean
}

/** Images added to the draft, saved by the engine as soon as they are added. */
export function useComposerImages() {
  const [images, setImages] = useState<ComposerImage[]>([])
  const imagesRef = useRef(images)
  imagesRef.current = images
  /** Being sent: their files stay even if the composer goes away meanwhile. */
  const sending = useRef(new Set<string>())

  const update = (key: string, change: Partial<ComposerImage>) =>
    setImages((current) =>
      current.map((image) =>
        image.key === key ? { ...image, ...change } : image
      )
    )

  const add = (files: File[]) => {
    for (const file of files) {
      const key = crypto.randomUUID()
      const image: ComposerImage = {
        key,
        name: file.name || 'image.png',
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
    }
  }

  const remove = (key: string) => {
    const image = imagesRef.current.find((candidate) => candidate.key === key)
    if (!image) return
    URL.revokeObjectURL(image.previewUrl)
    if (image.saved) void invoke('discard_image', { image: image.saved })
    setImages((current) => current.filter((candidate) => candidate.key !== key))
  }

  /** Keeps the files of the images being sent; the returned function undoes it. */
  const keep = () => {
    const keys = imagesRef.current.map((image) => image.key)
    for (const key of keys) sending.current.add(key)
    return () => {
      for (const key of keys) sending.current.delete(key)
    }
  }

  /** After sending: the files stay, the transcript shows them now. */
  const clear = () => {
    for (const image of imagesRef.current) URL.revokeObjectURL(image.previewUrl)
    setImages([])
  }

  useEffect(
    () => () => {
      for (const image of imagesRef.current) {
        URL.revokeObjectURL(image.previewUrl)
        if (image.saved && !sending.current.has(image.key)) {
          void invoke('discard_image', { image: image.saved })
        }
      }
    },
    []
  )

  return {
    images,
    add,
    remove,
    keep,
    clear,
    saving: images.some((image) => !image.saved && !image.failed),
    saved: images.flatMap((image) => (image.saved ? [image.saved] : []))
  }
}
