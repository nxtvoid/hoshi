import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import type { ThreadItem } from '@/bindings/ThreadItem'
import type { ToolCallContent, ToolCallItem } from '@/lib/tool-calls'

/** A page of the transcript leaves out long output and diffs, which the item alone has. */
function incomplete(content: ToolCallContent) {
  return (
    content.outputOmitted ||
    (content.detail?.kind === 'fileChange' && content.detail.diff === null)
  )
}

/** The call whole: as loaded when nothing was left out, otherwise read once its row opens. */
export function useWholeToolCall(
  item: ToolCallItem,
  open: boolean
): ToolCallContent {
  const [whole, setWhole] = useState<ToolCallContent | null>(null)
  const missing = incomplete(item.content)

  useEffect(() => {
    if (!open || !missing || whole !== null) return
    let current = true
    invoke<ThreadItem | null>('get_thread_item', {
      threadId: item.threadId,
      itemId: item.id
    })
      .then((loaded) => {
        if (current && loaded?.content.kind === 'toolCall') {
          setWhole(loaded.content)
        }
      })
      .catch((error) =>
        console.error('hoshi: failed to load a tool call', error)
      )
    return () => {
      current = false
    }
  }, [open, missing, whole, item.threadId, item.id])

  return missing && whole ? whole : item.content
}
