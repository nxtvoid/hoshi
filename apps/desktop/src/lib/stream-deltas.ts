import type { StreamingText } from './tool-calls'
import type { StreamDelta } from '@/bindings/StreamDelta'

/** What is still arriving: assistant text and reasoning, and running tools' output. */
export type LiveText = {
  streaming: Record<string, StreamingText>
  toolOutput: Record<string, string>
}

/**
 * Adds a delta to those not shown yet, by item id. Text is appended; a tool's output is
 * the whole output so far, so it replaces the one before.
 */
export function bufferDelta(
  buffer: Map<string, StreamingText>,
  { itemId, kind, delta }: StreamDelta
) {
  const previous = buffer.get(itemId)
  buffer.set(itemId, {
    kind,
    text: kind === 'toolOutput' || !previous ? delta : previous.text + delta
  })
}

/** The live text with the buffered deltas applied; unchanged records keep their identity. */
export function applyDeltas(
  live: LiveText,
  buffer: ReadonlyMap<string, StreamingText>
): LiveText {
  let { streaming, toolOutput } = live
  for (const [id, { kind, text }] of buffer) {
    if (kind === 'toolOutput') {
      if (toolOutput[id] === text) continue
      if (toolOutput === live.toolOutput) toolOutput = { ...toolOutput }
      toolOutput[id] = text
    } else {
      if (streaming === live.streaming) streaming = { ...streaming }
      streaming[id] = { kind, text: (streaming[id]?.text ?? '') + text }
    }
  }
  return streaming === live.streaming && toolOutput === live.toolOutput
    ? live
    : { streaming, toolOutput }
}
