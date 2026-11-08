import { useState } from 'react'
import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { useAppStore } from '@/state/app-store'
import { HugeiconsIcon } from '@hugeicons/react'
import { Button } from '@hoshi/ui/components/button'
import {
  Cancel01Icon,
  Tag01Icon,
  UserGroupIcon
} from '@hugeicons/core-free-icons'
import { LabelPicker, ReviewerPicker } from './pickers'
import type { PullRequest } from '@/bindings/PullRequest'
import { Markdown } from '@/components/markdown/Markdown'
import type { ReviewThread } from '@/bindings/ReviewThread'
import { formatReviewThreads } from '@/lib/review-comments'
import type { PullRequestActions } from './use-pull-request-actions'
import type { PullRequestDetail } from '@/bindings/PullRequestDetail'
import { Chip, REVIEW_STATE, SectionTitle, TextEditor } from './shared'

export function SummaryTab({
  threadId,
  pullRequest,
  detail,
  actions
}: {
  threadId: string
  pullRequest: PullRequest
  detail: PullRequestDetail
  actions: PullRequestActions
}) {
  const working = useAppStore(
    (state) =>
      state.threads.find((thread) => thread.id === threadId)?.status ===
      'working'
  )
  const sendMessage = useAppStore((state) => state.sendMessage)
  const unresolved = detail.reviewThreads.filter((thread) => !thread.resolved)
  const editable = detail.canEdit && pullRequest.state !== 'merged'

  return (
    <div className='flex flex-col gap-4 p-3'>
      <dl className='grid grid-cols-[6rem_1fr] items-start gap-x-3 gap-y-2 text-xs'>
        <dt className='text-muted-foreground flex h-6 items-center gap-1.5'>
          <HugeiconsIcon
            icon={UserGroupIcon}
            strokeWidth={2}
            className='size-3.5'
          />
          {m.pr_reviewers()}
        </dt>
        <dd className='flex flex-wrap items-center gap-1'>
          {detail.reviewers.length === 0 ? (
            <span className='text-muted-foreground'>{m.none()}</span>
          ) : null}
          {detail.reviewers.map((reviewer) => {
            const state = REVIEW_STATE[reviewer.state]
            const removable = editable && reviewer.state === 'requested'
            return (
              <Chip key={reviewer.name} title={state.label()}>
                <HugeiconsIcon
                  icon={state.icon}
                  strokeWidth={2}
                  className={cn('size-3', state.color)}
                />
                {reviewer.name}
                {removable ? (
                  <RemoveButton
                    label={m.remove_reviewer({ name: reviewer.name })}
                    disabled={actions.pending}
                    onClick={() =>
                      void actions.run({
                        type: 'removeReviewers',
                        names: [reviewer.name]
                      })
                    }
                  />
                ) : null}
              </Chip>
            )
          })}
          {editable ? (
            <ReviewerPicker
              threadId={threadId}
              current={detail.reviewers.map((reviewer) => reviewer.name)}
              author={pullRequest.author}
              actions={actions}
            />
          ) : null}
        </dd>
        {detail.supports.labels ? (
          <>
            <dt className='text-muted-foreground flex h-6 items-center gap-1.5'>
              <HugeiconsIcon
                icon={Tag01Icon}
                strokeWidth={2}
                className='size-3.5'
              />
              {m.pr_labels()}
            </dt>
            <dd className='flex flex-wrap items-center gap-1'>
              {detail.labels.length === 0 ? (
                <span className='text-muted-foreground'>{m.none()}</span>
              ) : null}
              {detail.labels.map((label) => (
                <Chip key={label.name}>
                  <span
                    className='size-2 rounded-full'
                    style={{ backgroundColor: `#${label.color}` }}
                  />
                  {label.name}
                  {editable ? (
                    <RemoveButton
                      label={m.remove_label({ name: label.name })}
                      disabled={actions.pending}
                      onClick={() =>
                        void actions.run({
                          type: 'removeLabel',
                          name: label.name
                        })
                      }
                    />
                  ) : null}
                </Chip>
              ))}
              {editable ? (
                <LabelPicker
                  threadId={threadId}
                  current={detail.labels.map((label) => label.name)}
                  actions={actions}
                />
              ) : null}
            </dd>
          </>
        ) : null}
      </dl>
      {unresolved.length > 0 ? (
        <section className='flex flex-col gap-2'>
          <div className='flex items-center gap-2'>
            <SectionTitle>
              {m.unresolved_comments({ count: unresolved.length })}
            </SectionTitle>
            <Button
              size='xs'
              className='ml-auto'
              disabled={working}
              onClick={() =>
                void sendMessage(
                  m.address_comments_prompt({
                    number: pullRequest.number,
                    comments: formatReviewThreads(unresolved)
                  })
                )
              }
            >
              {m.address_comments()}
            </Button>
          </div>
          <ul className='flex flex-col gap-2'>
            {unresolved.map((thread) => (
              <ReviewThreadItem
                key={thread.id}
                thread={thread}
                actions={actions}
              />
            ))}
          </ul>
        </section>
      ) : null}
      <Description
        pullRequest={pullRequest}
        detail={detail}
        editable={editable}
        actions={actions}
      />
    </div>
  )
}

function Description({
  pullRequest,
  detail,
  editable,
  actions
}: {
  pullRequest: PullRequest
  detail: PullRequestDetail
  editable: boolean
  actions: PullRequestActions
}) {
  const [editing, setEditing] = useState(false)

  return (
    <section className='flex flex-col gap-2'>
      <div className='flex items-center gap-2'>
        <SectionTitle>{m.pr_description()}</SectionTitle>
        {editable && !editing ? (
          <Button
            size='xs'
            variant='ghost'
            className='ml-auto'
            onClick={() => setEditing(true)}
          >
            {m.edit()}
          </Button>
        ) : null}
      </div>
      {editing ? (
        <TextEditor
          initial={detail.body}
          rows={14}
          saveLabel={m.save()}
          pending={actions.pending}
          onCancel={() => setEditing(false)}
          onSave={async (body) => {
            const saved = await actions.run({
              type: 'edit',
              title: pullRequest.title,
              body
            })
            if (saved) setEditing(false)
            return saved
          }}
        />
      ) : detail.body.trim() ? (
        <Markdown source='forge' text={detail.body} />
      ) : (
        <p className='text-muted-foreground text-xs'>{m.pr_no_description()}</p>
      )}
    </section>
  )
}

function ReviewThreadItem({
  thread,
  actions
}: {
  thread: ReviewThread
  actions: PullRequestActions
}) {
  const [replying, setReplying] = useState(false)

  return (
    <li className='border-border/60 bg-card/40 rounded-lg border text-xs'>
      <p className='border-border/60 text-muted-foreground border-b px-2.5 py-1.5 font-mono'>
        {thread.line === null ? thread.path : `${thread.path}:${thread.line}`}
      </p>
      <ul className='flex flex-col gap-2 px-2.5 py-2'>
        {thread.comments.map((comment) => (
          <li key={comment.url} className='flex flex-col gap-0.5'>
            <span className='font-medium'>{comment.author}</span>
            <Markdown source='forge' text={comment.body} className='text-xs' />
          </li>
        ))}
      </ul>
      <div className='border-border/60 flex flex-col gap-1.5 border-t px-2.5 py-1.5'>
        {replying ? (
          <TextEditor
            initial=''
            rows={3}
            placeholder={m.write_reply()}
            saveLabel={m.reply()}
            pending={actions.pending}
            onCancel={() => setReplying(false)}
            onSave={async (body) => {
              const sent = await actions.run({
                type: 'replyToThread',
                threadId: thread.id,
                body
              })
              if (sent) setReplying(false)
              return sent
            }}
          />
        ) : (
          <div className='flex justify-end gap-1.5'>
            <Button size='xs' variant='ghost' onClick={() => setReplying(true)}>
              {m.reply()}
            </Button>
            <Button
              size='xs'
              variant='outline'
              disabled={actions.pending}
              onClick={() =>
                void actions.run({
                  type: 'setThreadResolved',
                  threadId: thread.id,
                  resolved: true
                })
              }
            >
              {m.resolve()}
            </Button>
          </div>
        )}
      </div>
    </li>
  )
}

function RemoveButton({
  label,
  disabled,
  onClick
}: {
  label: string
  disabled: boolean
  onClick: () => void
}) {
  return (
    <button
      type='button'
      aria-label={label}
      disabled={disabled}
      onClick={onClick}
      className='text-muted-foreground hover:text-foreground focus-visible:ring-ring -mr-1 rounded-full p-0.5 outline-none focus-visible:ring-2'
    >
      <HugeiconsIcon icon={Cancel01Icon} strokeWidth={2} className='size-2.5' />
    </button>
  )
}
