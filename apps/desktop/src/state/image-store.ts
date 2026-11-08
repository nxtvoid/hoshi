import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'

/** Sent images as data URLs, read once each when the transcript first shows them. */
type ImageState = {
  /** `null` when the file is gone. */
  urls: Record<string, string | null>
  load: (id: string, mimeType: string) => void
}

const loading = new Set<string>()

export const useImageStore = create<ImageState>()((set, get) => ({
  urls: {},
  load: (id, mimeType) => {
    if (id in get().urls || loading.has(id)) return
    loading.add(id)
    invoke<string | null>('get_image', { id, mimeType })
      .then((url) => set((state) => ({ urls: { ...state.urls, [id]: url } })))
      .catch((error) => console.error('hoshi: failed to read an image', error))
      .finally(() => loading.delete(id))
  }
}))
