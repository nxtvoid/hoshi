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
export type SavedFile = Extract<MessageAttachment, { type: 'file' }>
/** An image or file the engine saved for a message. */
export type SavedAttachment = SavedImage | SavedFile

/** An image or file the draft carries, already saved by the engine. */
export type DraftAttachment = {
  /** The key its chip in the text names. */
  key: string
  attachment: SavedAttachment
  /** Shown as a chip in the text, which it goes with. */
  chipped: boolean
}

type DraftState = {
  /** Unsent composer text, as the editor's document, by draft key. */
  drafts: Record<string, JSONContent>
  /** Unsent images and files, by draft key. */
  attachments: Record<string, DraftAttachment[]>
  /** `null` forgets the draft's text. */
  save: (key: string, doc: JSONContent | null) => void
  /** An empty list forgets the draft's images and files. */
  saveAttachments: (key: string, attachments: DraftAttachment[]) => void
  /** Drops these drafts whole, with their images and files. */
  forget: (keys: string[]) => void
}

/** Whether the draft holds anything to send: text, attachments or both. */
export function hasDraft(
  state: Pick<DraftState, 'drafts' | 'attachments'>,
  key: string
) {
  return key in state.drafts || (state.attachments[key]?.length ?? 0) > 0
}

type PersistedDraft = Pick<DraftState, 'drafts' | 'attachments'>

/** Drafts saved before files could be attached kept only images, under `images`. */
function migrated(persisted: unknown, version: number): PersistedDraft {
  const state = (persisted ?? {}) as Partial<PersistedDraft> & {
    images?: Record<
      string,
      { key: string; image: SavedImage; chipped: boolean }[]
    >
  }
  if (version >= 1) {
    return { drafts: state.drafts ?? {}, attachments: state.attachments ?? {} }
  }
  const attachments: Record<string, DraftAttachment[]> = {}
  for (const [key, images] of Object.entries(state.images ?? {})) {
    attachments[key] = images.map(({ key, image, chipped }) => ({
      key,
      attachment: image,
      chipped
    }))
  }
  return { drafts: state.drafts ?? {}, attachments }
}

const storage = debouncedStorage<DraftState>(300)
flushOnLeave(storage.flush)

/** Saves the drafts now instead of a moment later. */
export const flushDrafts = storage.flush

export const useDraftStore = create<DraftState>()(
  persist(
    (set, get) => ({
      drafts: {},
      attachments: {},
      save: (key, doc) =>
        set((state) => {
          if (doc === null) {
            if (!(key in state.drafts)) return state
            const { [key]: _, ...rest } = state.drafts
            return { drafts: rest }
          }
          return { drafts: { ...state.drafts, [key]: doc } }
        }),
      saveAttachments: (key, attachments) =>
        set((state) => {
          if (attachments.length > 0) {
            return { attachments: { ...state.attachments, [key]: attachments } }
          }
          if (!(key in state.attachments)) return state
          const { [key]: _, ...rest } = state.attachments
          return { attachments: rest }
        }),
      forget: (keys) => {
        for (const key of keys) {
          for (const { attachment } of get().attachments[key] ?? []) {
            void invoke('discard_attachment', { attachment }).catch(() => {})
          }
        }
        set((state) => {
          const drafts = withoutKeys(state.drafts, keys)
          const attachments = withoutKeys(state.attachments, keys)
          return drafts === state.drafts && attachments === state.attachments
            ? state
            : { drafts, attachments }
        })
      }
    }),
    {
      name: 'hoshi-drafts',
      storage,
      version: 1,
      migrate: (persisted, version) =>
        migrated(persisted, version) as unknown as DraftState
    }
  )
)
