import type { ContextUsage } from '@/bindings/ContextUsage'

/** Below this, rereading the whole context costs little: Claude Code's own threshold. */
export const COMPACTION_MIN_TOKENS = 200_000

/** What makes the agent compact its conversation when sent as a message. */
export const COMPACT_COMMAND = '/compact'

/**
 * Whether sending now should compact first: the context is large and its prompt cache ran
 * out, so the next message would pay to read all of it again. Never while a turn runs.
 */
export function compactionDue(
  usage: ContextUsage | undefined,
  now: number,
  working: boolean
) {
  if (!usage || working) return false
  return (
    usage.usedTokens >= COMPACTION_MIN_TOKENS &&
    usage.cacheExpiresAt !== null &&
    now >= usage.cacheExpiresAt
  )
}

export function isCompactCommand(text: string) {
  return /^\/compact(?:\s|$)/i.test(text.trim())
}
