import { createContext, useContext } from 'react'
import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { ShieldUserIcon } from '@hugeicons/core-free-icons'
import type { ApprovalContent } from '@/lib/requests'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@hoshi/ui/components/tooltip'

/** The approvals of the transcript's tool calls, by the call they held back. */
export const ApprovalsContext = createContext<Map<string, ApprovalContent>>(
  new Map()
)

function describe(approval: ApprovalContent) {
  const tool = approval.toolName
  if (approval.status === 'pending') {
    return { label: m.request_waiting(), tone: 'text-status-waiting' }
  }
  if (approval.status === 'cancelled') {
    return { label: m.request_unanswered({ tool }), tone: '' }
  }
  switch (approval.decision) {
    case 'acceptForSession':
      return { label: m.request_allowed_session({ tool }), tone: '' }
    case 'decline':
      return { label: m.request_declined({ tool }), tone: 'text-destructive' }
    case 'cancel':
      return { label: m.request_stopped({ tool }), tone: 'text-destructive' }
    default:
      return { label: m.request_allowed({ tool }), tone: '' }
  }
}

/** Whether the user had to approve the call, and what they answered, at the end of its row. */
export function ApprovalBadge({ callId }: { callId: string }) {
  const approval = useContext(ApprovalsContext).get(callId)
  if (!approval) return null
  const { label, tone } = describe(approval)
  return (
    <Tooltip>
      <TooltipTrigger render={<span className='ms-auto flex shrink-0' />}>
        <HugeiconsIcon
          icon={ShieldUserIcon}
          strokeWidth={2}
          aria-label={label}
          className={cn('size-3.5', tone)}
        />
      </TooltipTrigger>
      <TooltipContent sideOffset={10}>
        {label}
        {approval.feedback ? (
          <span className='block opacity-80'>{approval.feedback}</span>
        ) : null}
      </TooltipContent>
    </Tooltip>
  )
}
