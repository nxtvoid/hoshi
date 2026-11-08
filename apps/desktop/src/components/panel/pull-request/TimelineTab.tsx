import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { openUrl } from '@tauri-apps/plugin-opener'
import { PanelMessage } from '../ChangedFiles'
import { Markdown } from '@/components/markdown/Markdown'
import type { TimelineItem } from '@/bindings/TimelineItem'
import type { PullRequestActions } from './use-pull-request-actions'
import type { PullRequestDetail } from '@/bindings/PullRequestDetail'
import { Comment01Icon, GitCommitIcon } from '@hugeicons/core-free-icons'
import { REVIEW_STATE, relativeTime, SectionTitle, TextEditor } from './shared'

export function TimelineTab({
  detail,
  canComment,
  actions
}: {
  detail: PullRequestDetail
  canComment: boolean
  actions: PullRequestActions
}) {
  return (
    <div className='flex flex-col gap-3 p-3'>
      {detail.timeline.length === 0 ? (
        <PanelMessage>{m.pr_no_activity()}</PanelMessage>
      ) : (
        <ol className='flex flex-col gap-3'>
          {detail.timeline.map((item) => (
            <TimelineEntry
              key={`${item.kind}:${item.url}:${item.createdAt}`}
              item={item}
            />
          ))}
        </ol>
      )}
      {canComment ? (
        <section className='border-border flex flex-col gap-1.5 border-t pt-3'>
          <SectionTitle>{m.add_comment()}</SectionTitle>
          <TextEditor
            initial=''
            rows={3}
            placeholder={m.write_comment()}
            saveLabel={m.comment()}
            pending={actions.pending}
            onSave={(body) => actions.run({ type: 'comment', body })}
          />
        </section>
      ) : null}
    </div>
  )
}

function TimelineEntry({ item }: { item: TimelineItem }) {
  if (item.kind === 'commit') {
    return (
      <li className='text-muted-foreground flex items-center gap-2 text-xs'>
        <HugeiconsIcon
          icon={GitCommitIcon}
          strokeWidth={2}
          className='size-3.5 shrink-0'
        />
        <span className='text-foreground min-w-0 flex-1 truncate'>
          {item.message}
        </span>
        <button
          type='button'
          onClick={() => item.url && void openUrl(item.url)}
          className='hover:text-foreground shrink-0 font-mono'
        >
          {item.sha}
        </button>
        <span className='shrink-0'>{relativeTime(item.createdAt)}</span>
      </li>
    )
  }
  const review = item.kind === 'review' ? REVIEW_STATE[item.state] : null
  return (
    <li className='border-border/60 bg-card/40 rounded-lg border text-xs'>
      <div className='border-border/60 text-muted-foreground flex items-center gap-1.5 border-b px-2.5 py-1.5'>
        <HugeiconsIcon
          icon={review?.icon ?? Comment01Icon}
          strokeWidth={2}
          className={cn('size-3.5', review?.color)}
        />
        <span className='text-foreground font-medium'>{item.author}</span>
        {review ? <span>{review.label()}</span> : null}
        <span className='ml-auto'>{relativeTime(item.createdAt)}</span>
      </div>
      {item.body.trim() ? (
        <Markdown
          source='forge'
          text={item.body}
          className='px-2.5 py-2 text-xs'
        />
      ) : null}
    </li>
  )
}
