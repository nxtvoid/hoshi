import { create } from 'zustand'
import { withoutKeys } from '@/lib/records'
import { persist } from 'zustand/middleware'
import type { JSONContent } from '@tiptap/core'
import { invoke } from '@tauri-apps/api/core'
import type { MessageAttachment } from '@/bindings/MessageAttachment'
import { debouncedStorage, flushOnLeave } from './debounced-storage'

/** Where a draft belongs: a thread, or a thread not started yet in a project or among the chats. */
export function draftKey(
  threadId: string | null,
  projectId: string | null,
  chat: boolean
) {
  if (threadId) return threadId
  return chat ? 'new:chat' : `new:${projectId ?? ''}`
}

export type SavedImage = Extract<MessageAttachment, { type: 'image' }>

/** An image the draft carries, already saved by the engine. */
export type DraftImage = {
  /** The key its chip in the text names. */
  key: string
  image: SavedImage
  /** Shown as a chip in the text, which it goes with. */
  chipped: boolean
}

type DraftState = {
  /** Unsent composer text, as the editor's document, by draft key. */
  drafts: Record<string, JSONContent>
  /** Unsent images, by draft key. */
  images: Record<string, DraftImage[]>
  /** `null` forgets the draft's text. */
  save: (key: string, doc: JSONContent | null) => void
  /** An empty list forgets the draft's images. */
  saveImages: (key: string, images: DraftImage[]) => void
  /** Drops these drafts whole, with the files of their images. */
  forget: (keys: string[]) => void
}

/** Whether the draft holds anything to send: text, images or both. */
export function hasDraft(
  state: Pick<DraftState, 'drafts' | 'images'>,
  key: string
) {
  return key in state.drafts || (state.images[key]?.length ?? 0) > 0
}

const storage = debouncedStorage<DraftState>(300)
flushOnLeave(storage.flush)

/** Saves the drafts now instead of a moment later. */
export const flushDrafts = storage.flush

export const useDraftStore = create<DraftState>()(
  persist(
    (set, get) => ({
      drafts: {},
      images: {},
      save: (key, doc) =>
        set((state) => {
          if (doc === null) {
            if (!(key in state.drafts)) return state
            const { [key]: _, ...rest } = state.drafts
            return { drafts: rest }
          }
          return { drafts: { ...state.drafts, [key]: doc } }
        }),
      saveImages: (key, images) =>
        set((state) => {
          if (images.length > 0) {
            return { images: { ...state.images, [key]: images } }
          }
          if (!(key in state.images)) return state
          const { [key]: _, ...rest } = state.images
          return { images: rest }
        }),
      forget: (keys) => {
        for (const key of keys) {
          for (const { image } of get().images[key] ?? []) {
            void invoke('discard_image', { image }).catch(() => {})
          }
        }
        set((state) => {
          const drafts = withoutKeys(state.drafts, keys)
          const images = withoutKeys(state.images, keys)
          return drafts === state.drafts && images === state.images
            ? state
            : { drafts, images }
        })
      }
    }),
    { name: 'hoshi-drafts', storage }
  )
)
