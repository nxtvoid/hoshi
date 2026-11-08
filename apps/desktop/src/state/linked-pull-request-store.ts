import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import type { PullRequest } from '@/bindings/PullRequest'

type LinkedPullRequestState = {
  /** The pull request each link points to, by URL; `null` when it can't be read. */
  byUrl: Record<string, PullRequest | null>
  /** Reads the link's pull request again; the engine answers recent reads from memory. */
  load: (url: string) => void
}

const loading = new Set<string>()

export const useLinkedPullRequestStore = create<LinkedPullRequestState>()(
  (set) => ({
    byUrl: {},
    load: (url) => {
      if (loading.has(url)) return
      loading.add(url)
      invoke<PullRequest | null>('get_linked_pull_request', { url })
        .catch((error) => {
          console.error('hoshi: failed to read a linked pull request', error)
          return null
        })
        .then((pullRequest) =>
          set((state) => ({
            byUrl: {
              ...state.byUrl,
              [url]: pullRequest ?? state.byUrl[url] ?? null
            }
          }))
        )
        .finally(() => loading.delete(url))
    }
  })
)
