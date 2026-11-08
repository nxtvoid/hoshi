import type { ReviewThread } from '@/bindings/ReviewThread'

/** The unresolved review threads as plain text for the agent: where each one is, then who said what. */
export function formatReviewThreads(threads: ReviewThread[]): string {
  return threads
    .filter((thread) => !thread.resolved && thread.comments.length > 0)
    .map((thread) => {
      const where =
        thread.line === null ? thread.path : `${thread.path}:${thread.line}`
      const comments = thread.comments.map((comment) => {
        const body = comment.body.trim().replaceAll('\n', '\n    ')
        return `  - ${comment.author ?? '?'}: ${body}`
      })
      return [`- ${where}`, ...comments].join('\n')
    })
    .join('\n')
}
