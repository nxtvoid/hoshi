import { useState } from 'react'
import { m } from '@/paraglide/messages'
import { subagentRun } from '@/lib/model-runs'
import { subagentState } from '@/lib/agents'
import { useAppStore } from '@/state/app-store'
import { AgentLine, AgentStatus } from './AgentLine'
import { useModelsStore } from '@/state/models-store'
import { useSubagentStore } from '@/state/subagent-store'
import { useTranscriptView } from './transcript-scope'
import {
  reasoningPreview,
  subagentInput,
  type ToolCallItem
} from '@/lib/tool-calls'

/**
 * A subagent the agent started: what it does or found, what it runs on and how it stands.
 * It opens the subagent's own transcript.
 */
export function SubagentRow({
  item,
  grouped = false
}: {
  item: ToolCallItem
  /** Inside a group's card, which draws the border. */
  grouped?: boolean
}) {
  const [stopping, setStopping] = useState(false)
  const { output, input, interrupted, detail } = item.content
  const state = subagentState(item.content)
  const running = state === 'running'
  const { description, kind } = subagentInput(input)
  const catalog = useModelsStore((state) => state.catalog)
  const progress = useTranscriptView((view) =>
    running ? view.toolOutput[item.id] : undefined
  )
  const stopSubagent = useAppStore((state) => state.stopSubagent)
  const view = useSubagentStore((state) => state.view)
  const title = description || m.subagent()

  return (
    <AgentLine
      grouped={grouped}
      state={state}
      title={title}
      kind={kind}
      detail={
        running
          ? progress
          : output && !interrupted
            ? reasoningPreview(output, false)
            : undefined
      }
      failed={state === 'failed'}
      run={subagentRun(catalog, detail)}
      status={
        <AgentStatus state={state} startedAt={item.createdAt} endedAt={null} />
      }
      label={m.delegation_open_task({ task: title })}
      onOpen={() => view(item.id)}
      onStop={
        running
          ? () => {
              setStopping(true)
              void stopSubagent(item.id)
            }
          : undefined
      }
      stopping={stopping}
    />
  )
}
