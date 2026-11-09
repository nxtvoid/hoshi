import { useEffect, useId, useRef, useState } from 'react'
import { cn } from '@hoshi/utils'
import { imageFiles } from '@/lib/images'
import { CHAT_LANE } from '@/lib/chat-lane'
import { m } from '@/paraglide/messages'
import { useAppStore } from '@/state/app-store'
import { useDraftStore } from '@/state/draft-store'
import { useImageStore } from '@/state/image-store'
import { useComposerFillStore } from '@/state/composer-fill-store'
import { useInChat, useScopeProjectId } from '@/state/use-scope'
import { HugeiconsIcon } from '@hugeicons/react'
import { ModelPicker } from './ModelPicker'
import { stepHistory } from '@/lib/prompt-history'
import { ContextMeter } from './ContextMeter'
import {
  DelegatedRequestPanel,
  PlanningTasksNote,
  RequestPanel
} from './RequestPanel'
import { ComposerImages } from './ComposerImages'
import { Button } from '@hoshi/ui/components/button'
import { useComposerImages } from './use-composer-images'
import type { CommandError } from '@/bindings/CommandError'
import { useModelPickerStore } from '@/state/model-picker-store'
import { usePromptHistoryStore } from '@/state/prompt-history-store'
import {
  AccessMenu,
  PlanToggle,
  useAccess,
  WorkspaceMenu
} from './ComposerMenus'
import { ComposerSuggestions, suggestionOptionId } from './ComposerSuggestions'
import { usePendingRequest, useRequestActions } from './use-request-actions'
import {
  ArrowUp02Icon,
  Attachment01Icon,
  BubbleChatIcon,
  Folder01Icon,
  StopIcon
} from '@hugeicons/core-free-icons'
import {
  ComposerEditor,
  type ComposerEditorHandle,
  type ComposerSnapshot,
  type KeyContext,
  type LocatedTrigger
} from './ComposerEditor'
import {
  type ComposerAction,
  type Suggestion,
  useComposerSuggestions
} from './use-composer-suggestions'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@hoshi/ui/components/tooltip'

type ComposerProps = {
  working: boolean
  /** Centered on an empty thread, with room to write; docked below a transcript. */
  variant: 'hero' | 'docked'
  /** Where unsent text is kept, so each thread and each new thread has its own. */
  draftKey: string
  /** The thread's working directory, to show paths relative to it. */
  cwd?: string | null
}

const IMAGE_TYPES = 'image/png,image/jpeg,image/gif,image/webp,image/*'

/** As tall as one line of the docked editor, so its buttons center on it. */
const DOCKED_BUTTON = 'flex h-8 shrink-0 items-center'
/** Past one line, the docked editor takes the whole width and its buttons go below. */
const ONE_LINE_PX = 40

const EMPTY: ComposerSnapshot = {
  text: '',
  empty: true,
  trigger: null,
  focused: true
}

const sameTrigger = (a: LocatedTrigger, b: LocatedTrigger | null) =>
  b !== null && a.kind === b.kind && a.from === b.from

export function Composer({
  working,
  variant,
  draftKey,
  cwd = null
}: ComposerProps) {
  const saveDraft = useDraftStore((state) => state.save)
  const [initialDraft] = useState(
    () => useDraftStore.getState().drafts[draftKey]
  )
  const [snapshot, setSnapshot] = useState(EMPTY)
  const [dismissed, setDismissed] = useState<LocatedTrigger | null>(null)
  const [highlight, setHighlight] = useState<{ id: string; key: string }>()
  /** The past prompt shown, while it is not edited. */
  const [recalled, setRecalled] = useState<{ position: number; text: string }>()
  const [dragging, setDragging] = useState(false)
  const [sending, setSending] = useState(false)
  /** The docked editor outgrew one line; it stays wide until emptied. */
  const [tall, setTall] = useState(false)
  const editorBoxRef = useRef<HTMLDivElement>(null)
  const editorRef = useRef<ComposerEditorHandle>(null)
  const fileInputRef = useRef<HTMLInputElement>(null)
  const listId = useId()
  const images = useComposerImages()
  const history = usePromptHistoryStore((state) => state.entries)
  const remember = usePromptHistoryStore((state) => state.add)
  const sendMessage = useAppStore((state) => state.sendMessage)
  const interruptTurn = useAppStore((state) => state.interruptTurn)
  const openModelPicker = useModelPickerStore((state) => state.setOpen)
  const [, updateAccess] = useAccess()
  const threadId = useAppStore((state) => state.selectedThreadId)
  const fill = useComposerFillStore((state) => state.fill)
  const restoreImages = images.restore

  useEffect(() => {
    if (!fill || fill.threadId !== threadId) return
    editorRef.current?.setText(fill.text)
    restoreImages(fill.attachments, useImageStore.getState().urls)
    useComposerFillStore.getState().done()
  }, [fill, threadId, restoreImages])
  const request = usePendingRequest()
  const requestActions = useRequestActions(request)
  const { text } = snapshot
  const answering = request !== null && text.trim() !== ''
  const written = text.trim() !== '' || images.saved.length > 0
  /** A message sent while the agent works joins its turn. */
  const canSend = request
    ? answering && !sending
    : written && !images.saving && !sending
  const hero = variant === 'hero'
  const stacked = hero || tall

  const detected = snapshot.focused ? snapshot.trigger : null
  const trigger =
    detected && !sameTrigger(detected, dismissed) ? detected : null
  const suggestions = useComposerSuggestions(trigger)
  const triggerKey = trigger
    ? `${trigger.kind}:${trigger.from}:${trigger.query}`
    : ''
  const activeId =
    highlight?.key === triggerKey &&
    suggestions.items.some((item) => item.id === highlight.id)
      ? highlight.id
      : (suggestions.items[0]?.id ?? null)
  const activeIndex = suggestions.items.findIndex(
    (item) => item.id === activeId
  )
  const menuOpen = trigger !== null && suggestions.items.length > 0

  useEffect(() => {
    editorRef.current?.setMenu({
      listId: menuOpen ? listId : null,
      activeId:
        menuOpen && activeIndex >= 0
          ? suggestionOptionId(listId, activeIndex)
          : null
    })
  }, [menuOpen, listId, activeIndex])

  useEffect(() => {
    const box = editorBoxRef.current
    if (!box) return
    const observer = new ResizeObserver(() => {
      if (box.offsetHeight > ONE_LINE_PX) setTall(true)
    })
    observer.observe(box)
    return () => observer.disconnect()
  }, [])

  const onChange = (next: ComposerSnapshot) => {
    setSnapshot(next)
    if (!next.trigger) setDismissed(null)
    if (next.empty) setTall(false)
  }

  const runAction = (action: ComposerAction) => {
    if (action === 'model') openModelPicker(true)
    else updateAccess({ planMode: action === 'plan' })
  }

  const pick = (item: Suggestion) => {
    const editor = editorRef.current
    if (!trigger || !editor) return
    switch (item.type) {
      case 'path': {
        const { path, kind } = item.entry
        if (kind === 'directory' && !/\s/.test(path)) {
          editor.replace(trigger, { text: `@${path}/`, keepOpen: true })
        } else {
          editor.replace(trigger, {
            mention: { kind: 'file', value: path, attachment: null }
          })
        }
        return
      }
      case 'pullRequest': {
        const { number, title, url, headBranch, baseBranch, state } =
          item.pullRequest
        editor.replace(trigger, {
          mention: {
            kind: 'pullRequest',
            value: String(number),
            attachment: {
              type: 'pullRequest',
              number,
              title,
              url,
              headBranch,
              baseBranch,
              state
            }
          }
        })
        return
      }
      case 'skill': {
        const { name } = item.command
        editor.replace(trigger, {
          mention: {
            kind: 'skill',
            value: name,
            attachment: { type: 'skill', name }
          }
        })
        return
      }
      case 'command':
        if (item.action) {
          editor.replace(trigger, { remove: true })
          runAction(item.action)
        } else {
          editor.replace(trigger, { text: `/${item.command.name}` })
        }
    }
  }

  /** Typed text answers the pending request: feedback that declines, or an own answer. */
  const answerRequest = async () => {
    if (!request || !canSend) return
    setSending(true)
    const answered =
      request.content.kind === 'questions'
        ? requestActions.advance(text)
        : await requestActions.decide('decline', text)
    setSending(false)
    if (!answered) return
    saveDraft(draftKey, null)
    editorRef.current?.setText('')
  }

  const submit = async () => {
    const editor = editorRef.current
    if (!editor) return
    if (request) {
      await answerRequest()
      return
    }
    const action = /^\/(\w+)$/.exec(text.trim())?.[1]
    const standalone = suggestions.actions.find((known) => known === action)
    if (standalone && images.images.length === 0) {
      editor.setText('')
      runAction(standalone)
      return
    }
    if (!canSend) return
    const attachments = [...images.saved, ...editor.mentions()]
    setSending(true)
    const release = images.keep()
    const sent = await sendMessage(text, attachments)
    setSending(false)
    if (!sent) {
      release()
      return
    }
    remember(text)
    saveDraft(draftKey, null)
    setRecalled(undefined)
    images.clear()
    editorRef.current?.setText('')
  }

  const recall = (direction: 'back' | 'forward') => {
    const showing = recalled && recalled.text === text ? recalled : undefined
    const step = stepHistory(history, showing?.position ?? null, direction)
    if (!step) return false
    setRecalled(
      step.position === null
        ? undefined
        : { position: step.position, text: step.text }
    )
    editorRef.current?.setText(step.text)
    return true
  }

  const onKeyDown = (event: KeyboardEvent, context: KeyContext) => {
    if (event.isComposing) return false
    const { items } = suggestions
    if (trigger) {
      if (
        (event.key === 'ArrowDown' || event.key === 'ArrowUp') &&
        items.length > 0
      ) {
        const step = event.key === 'ArrowDown' ? 1 : -1
        const next = items[(activeIndex + step + items.length) % items.length]
        if (next) setHighlight({ id: next.id, key: triggerKey })
        return true
      }
      const active = items[activeIndex]
      if (
        active &&
        (event.key === 'Tab' || (event.key === 'Enter' && !event.shiftKey))
      ) {
        pick(active)
        return true
      }
      if (event.key === 'Escape') {
        event.stopPropagation()
        setDismissed(trigger)
        return true
      }
    }

    const plain =
      !event.shiftKey && !event.altKey && !event.ctrlKey && !event.metaKey
    const showingRecalled = recalled !== undefined && recalled.text === text
    if (
      plain &&
      event.key === 'ArrowUp' &&
      (snapshot.empty || showingRecalled) &&
      context.onFirstLine()
    ) {
      return recall('back')
    }
    if (
      plain &&
      event.key === 'ArrowDown' &&
      showingRecalled &&
      context.onLastLine()
    ) {
      return recall('forward')
    }

    if (event.key === 'Enter') {
      const send =
        event.ctrlKey ||
        event.metaKey ||
        (!event.shiftKey && !event.altKey && !context.inCodeBlock)
      if (send) {
        void submit()
        return true
      }
    }
    return false
  }

  const attachButton = (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            type='button'
            size='icon-sm'
            variant='ghost'
            aria-label={m.composer_attach_images()}
            className='text-muted-foreground shrink-0 rounded-full'
            onClick={() => fileInputRef.current?.click()}
          />
        }
      >
        <HugeiconsIcon icon={Attachment01Icon} strokeWidth={2} />
      </TooltipTrigger>
      <TooltipContent>{m.composer_attach_images()}</TooltipContent>
    </Tooltip>
  )

  const action =
    working && !answering && !written ? (
      <Button
        type='button'
        size='icon-sm'
        variant='secondary'
        aria-label={m.stop_turn()}
        className='shrink-0 rounded-full'
        onClick={() => void interruptTurn()}
      >
        <HugeiconsIcon icon={StopIcon} strokeWidth={2} />
      </Button>
    ) : (
      <Button
        type='submit'
        size='icon-sm'
        aria-label={m.send_message()}
        className='shrink-0 rounded-full'
        disabled={!canSend}
      >
        <HugeiconsIcon icon={ArrowUp02Icon} strokeWidth={2} />
      </Button>
    )

  return (
    <div className={cn(CHAT_LANE, 'flex flex-col gap-1.5')}>
      <div className='relative'>
        {trigger ? (
          <ComposerSuggestions
            listId={listId}
            kind={trigger.kind}
            items={suggestions.items}
            activeId={activeId}
            status={suggestions.status}
            onHighlight={(id) => setHighlight({ id, key: triggerKey })}
            onPick={pick}
          />
        ) : null}
        {request ? (
          <RequestPanel
            request={request}
            cwd={cwd}
            onKeepPlanning={() => editorRef.current?.focus()}
          />
        ) : threadId ? (
          <>
            <DelegatedRequestPanel threadId={threadId} cwd={cwd} />
            <PlanningTasksNote threadId={threadId} />
          </>
        ) : null}
        <form
          className={cn(
            'border-border bg-card focus-within:border-ring/60 relative flex flex-col rounded-2xl border shadow-xs transition-colors',
            hero ? 'p-2' : 'py-1.5 pr-2 pl-1',
            dragging && 'border-ring ring-ring/30 ring-3'
          )}
          onSubmit={(event) => {
            event.preventDefault()
            void submit()
          }}
          onDragOver={(event) => {
            if (!event.dataTransfer.types.includes('Files')) return
            event.preventDefault()
            setDragging(true)
          }}
          onDragLeave={(event) => {
            const next = event.relatedTarget
            if (!(next instanceof Node && event.currentTarget.contains(next))) {
              setDragging(false)
            }
          }}
          onDrop={(event) => {
            event.preventDefault()
            setDragging(false)
            images.add(imageFiles(event.dataTransfer))
          }}
        >
          {images.images.length > 0 ? (
            <ComposerImages images={images.images} onRemove={images.remove} />
          ) : null}
          <div
            className={cn(
              'flex',
              stacked ? 'flex-col gap-1' : 'items-end gap-1'
            )}
          >
            {stacked ? null : (
              <div className={DOCKED_BUTTON}>{attachButton}</div>
            )}
            <div ref={editorBoxRef} className='min-w-0 flex-1'>
              <ComposerEditor
                ref={editorRef}
                placeholder={placeholder(request?.content.kind, working)}
                label={m.composer_label()}
                className='max-h-60 overflow-y-auto'
                contentClassName={hero ? 'min-h-16 py-2' : 'min-h-8 py-1.5'}
                onChange={onChange}
                initialContent={initialDraft}
                onDocument={(doc) => saveDraft(draftKey, doc)}
                onKeyDown={onKeyDown}
                onFiles={images.add}
              />
            </div>
            {stacked ? (
              <div className='flex items-center justify-between'>
                {attachButton}
                {action}
              </div>
            ) : (
              <div className={DOCKED_BUTTON}>{action}</div>
            )}
          </div>
          <input
            ref={fileInputRef}
            type='file'
            accept={IMAGE_TYPES}
            multiple
            hidden
            onChange={(event) => {
              images.add(imageFiles(event.currentTarget))
              event.currentTarget.value = ''
            }}
          />
        </form>
      </div>
      <ComposerFooter />
    </div>
  )
}

function ComposerFooter() {
  const chat = useInChat()
  const projectId = useScopeProjectId()
  const projectTitle = useAppStore(
    (state) => state.projects.find((project) => project.id === projectId)?.title
  )
  const hasThread = useAppStore((state) => state.selectedThreadId !== null)
  const error = useAppStore((state) => state.composerError)
  const place = chat ? m.chat() : projectTitle

  return (
    <div className='text-muted-foreground flex items-center gap-3 px-2 text-xs'>
      {place ? (
        <span className='flex min-w-0 items-center gap-1.5'>
          <HugeiconsIcon
            icon={chat ? BubbleChatIcon : Folder01Icon}
            strokeWidth={2}
            className='size-3.5 shrink-0'
          />
          <span className='truncate'>{place}</span>
        </span>
      ) : null}
      {error ? (
        <span role='alert' className='text-destructive min-w-0 truncate'>
          {commandErrorMessage(error)}
        </span>
      ) : null}
      <div className='ml-auto flex shrink-0 items-center gap-1'>
        {hasThread || chat ? null : <WorkspaceMenu />}
        <PlanToggle />
        <AccessMenu />
        <ContextMeter />
        <ModelPicker />
      </div>
    </div>
  )
}

function placeholder(
  request: 'approval' | 'questions' | 'proposedPlan' | undefined,
  working: boolean
) {
  switch (request) {
    case 'approval':
      return m.request_approval_placeholder()
    case 'questions':
      return m.request_question_placeholder()
    case 'proposedPlan':
      return m.request_plan_placeholder()
    case undefined:
      return working ? m.composer_steer_placeholder() : m.composer_placeholder()
  }
}

function commandErrorMessage(error: CommandError) {
  switch (error.code) {
    case 'notAGitRepository':
      return m.error_not_a_git_repository()
    case 'worktreeFailed':
      return m.error_worktree_failed()
    case 'threadBusy':
      return m.error_thread_busy()
    case 'requestNotPending':
      return m.error_request_not_pending()
    default:
      return m.error_generic()
  }
}
