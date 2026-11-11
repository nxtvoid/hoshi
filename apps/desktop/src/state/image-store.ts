import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'

/** Sent images as data URLs, read once each when the transcript first shows them. */
type ImageState = {
  /** `null` when the file is gone. */
  urls: Record<string, string | null>
  /** Settles once the image was read, or turned out to be gone. */
  load: (id: string, mimeType: string) => Promise<void>
  /** An image this window just saved, so showing it waits for nothing. */
  keep: (id: string, url: string) => void
  forget: (id: string) => void
}

const loading = new Map<string, Promise<void>>()

export const useImageStore = create<ImageState>()((set, get) => ({
  urls: {},
  load: (id, mimeType) => {
    if (id in get().urls) return Promise.resolve()
    const running = loading.get(id)
    if (running) return running
    const reading = invoke<string | null>('get_image', { id, mimeType })
      .then((url) => set((state) => ({ urls: { ...state.urls, [id]: url } })))
      .catch((error) => console.error('hoshi: failed to read an image', error))
      .finally(() => {
        loading.delete(id)
      })
    loading.set(id, reading)
    return reading
  },
  keep: (id, url) => set((state) => ({ urls: { ...state.urls, [id]: url } })),
  forget: (id) =>
    set((state) => {
      if (!(id in state.urls)) return state
      const { [id]: _, ...urls } = state.urls
      return { urls }
    })
}))
