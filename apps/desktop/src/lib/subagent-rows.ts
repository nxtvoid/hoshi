import type { ThreadItem } from '@/bindings/ThreadItem'
import {
  foldTurns,
  groupTranscript,
  type ToolCallItem,
  type TranscriptRow
} from './tool-calls'

/**
 * A subagent's transcript as rows, like a thread's turn: its work folds once it reported,
 * and its report comes last. `report` is what its call returned, shown unless its last
 * message already says it.
 */
export function subagentRows(
  call: ToolCallItem,
  items: ThreadItem[],
  report: string | null,
  running: boolean
): TranscriptRow[] {
  const threadId = items[0]?.threadId ?? call.threadId
  const said = items
    .filter((item) => item.content.kind === 'assistantMessage')
    .at(-1)
  const repeated =
    said?.content.kind === 'assistantMessage' &&
    said.content.text.trim() === report?.trim()
  const asked: ThreadItem = {
    id: `${call.id}:asked`,
    threadId,
    createdAt: call.createdAt,
    content: {
      kind: 'userMessage',
      text: '',
      attachments: [],
      steer: false,
      taskResult: null,
      pullRequestUpdate: null,
      rewind: null
    }
  }
  const reported: ThreadItem[] =
    report && !repeated
      ? [
          {
            id: `${call.id}:report`,
            threadId,
            createdAt: items.at(-1)?.createdAt ?? call.createdAt,
            content: { kind: 'assistantMessage', text: report }
          }
        ]
      : []
  return foldTurns(
    groupTranscript([asked, ...items, ...reported]),
    running
  ).filter((row) => row.kind !== 'item' || row.item.id !== asked.id)
}
