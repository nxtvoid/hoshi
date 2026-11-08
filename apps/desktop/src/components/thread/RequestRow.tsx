import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { toolTarget } from '@/lib/tool-calls'
import { HugeiconsIcon } from '@hugeicons/react'
import type { RequestContent } from '@/lib/requests'
import { Markdown } from '@/components/markdown/Markdown'
import {
  ArrowRight01Icon,
  Cancel01Icon,
  Clock01Icon,
  Tick02Icon,
  UserQuestion01Icon
} from '@hugeicons/core-free-icons'

const SUMMARY =
  'text-muted-foreground hover:text-foreground flex min-h-6 cursor-default list-none items-center gap-2 text-sm select-none [&::-webkit-details-marker]:hidden'

/** A request in the transcript, once answered or while its panel waits for the user. */
export function RequestRow({
  content,
  cwd
}: {
  content: RequestContent
  cwd: string | null
}) {
  switch (content.kind) {
    case 'approval':
      return <ApprovalRow content={content} cwd={cwd} />
    case 'questions':
      return <QuestionsRow content={content} />
    case 'proposedPlan':
      return <PlanRow content={content} />
  }
}

function StatusIcon({ content }: { content: RequestContent }) {
  const declined =
    content.kind !== 'questions' &&
    (content.decision === 'decline' || content.decision === 'cancel')
  const icon =
    content.status === 'pending'
      ? Clock01Icon
      : content.status === 'cancelled' || declined
        ? Cancel01Icon
        : Tick02Icon
  return (
    <HugeiconsIcon
      icon={icon}
      strokeWidth={2}
      className={cn(
        'size-4 shrink-0',
        content.status === 'pending' && 'text-status-waiting'
      )}
    />
  )
}

function ApprovalRow({
  content,
  cwd
}: {
  content: Extract<RequestContent, { kind: 'approval' }>
  cwd: string | null
}) {
  const tool = content.toolName
  const label =
    content.status === 'pending'
      ? m.request_approval_title({ tool })
      : content.status === 'cancelled'
        ? m.request_unanswered({ tool })
        : content.decision === 'acceptForSession'
          ? m.request_allowed_session({ tool })
          : content.decision === 'decline'
            ? m.request_declined({ tool })
            : content.decision === 'cancel'
              ? m.request_stopped({ tool })
              : m.request_allowed({ tool })
  return (
    <div className='flex flex-col gap-1'>
      <div className='text-muted-foreground flex min-h-6 items-center gap-2 text-sm'>
        <StatusIcon content={content} />
        <span className='text-foreground shrink-0 font-medium'>{label}</span>
        <span className='min-w-0 truncate font-mono text-xs'>
          {toolTarget(tool, content.input, cwd)}
        </span>
      </div>
      {content.feedback ? (
        <p className='text-muted-foreground border-border ms-6 border-s ps-2.5 text-sm'>
          {content.feedback}
        </p>
      ) : null}
    </div>
  )
}

function QuestionsRow({
  content
}: {
  content: Extract<RequestContent, { kind: 'questions' }>
}) {
  const { status, questions, answers } = content
  return (
    <div className='flex flex-col gap-1.5'>
      <div className='text-muted-foreground flex min-h-6 items-center gap-2 text-sm'>
        {status === 'resolved' ? (
          <HugeiconsIcon
            icon={UserQuestion01Icon}
            strokeWidth={2}
            className='size-4 shrink-0'
          />
        ) : (
          <StatusIcon content={content} />
        )}
        <span className='text-foreground font-medium'>
          {status === 'pending'
            ? m.request_waiting()
            : status === 'cancelled'
              ? m.request_questions_unanswered()
              : m.request_answered()}
        </span>
      </div>
      <dl className='border-border ms-2 flex flex-col gap-1.5 border-s ps-4 text-sm'>
        {questions.map((question, index) => (
          <div key={`${index}:${question.question}`}>
            <dt className='text-muted-foreground'>{question.question}</dt>
            {answers?.[index] ? (
              <dd className='text-foreground'>{answers[index]}</dd>
            ) : null}
          </div>
        ))}
      </dl>
    </div>
  )
}

function PlanRow({
  content
}: {
  content: Extract<RequestContent, { kind: 'proposedPlan' }>
}) {
  const label =
    content.status === 'pending'
      ? m.request_plan_title()
      : content.status === 'cancelled'
        ? m.request_plan_unanswered()
        : content.decision === 'accept'
          ? m.request_plan_accepted()
          : m.request_plan_sent_back()
  return (
    <details className='group/plan'>
      <summary className={SUMMARY}>
        <StatusIcon content={content} />
        <span className='text-foreground shrink-0 font-medium'>{label}</span>
        <HugeiconsIcon
          icon={ArrowRight01Icon}
          strokeWidth={2}
          className='size-3.5 shrink-0 opacity-0 group-open/plan:rotate-90 group-open/plan:opacity-100 group-hover/plan:opacity-100'
        />
      </summary>
      <div className='bg-muted/40 ms-6 mt-1 mb-2 max-h-96 overflow-auto rounded-md px-3 py-2'>
        <Markdown text={content.plan} className='text-sm' />
      </div>
      {content.feedback ? (
        <p className='text-muted-foreground border-border ms-6 border-s ps-2.5 text-sm'>
          {content.feedback}
        </p>
      ) : null}
    </details>
  )
}
