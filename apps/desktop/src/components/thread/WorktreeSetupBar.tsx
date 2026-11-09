import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { CHAT_LANE } from '@/lib/chat-lane'
import { invoke } from '@tauri-apps/api/core'
import { HugeiconsIcon } from '@hugeicons/react'
import { Button } from '@hoshi/ui/components/button'
import { useTerminalStore } from '@/state/terminal-store'
import { useWorktreeSetupStore } from '@/state/worktree-setup-store'
import type { WorktreeSetupRun } from '@/bindings/WorktreeSetupRun'
import {
  Alert02Icon,
  Cancel01Icon,
  CheckmarkCircle02Icon,
  Loading03Icon
} from '@hugeicons/core-free-icons'

function headline(run: WorktreeSetupRun) {
  if (run.status === 'running') return m.worktree_setup_running()
  if (run.status === 'succeeded') return m.worktree_setup_succeeded()
  return run.exitCode === null
    ? m.worktree_setup_failed()
    : m.worktree_setup_failed_code({ code: run.exitCode })
}

/** Above the composer while the project's setup command runs in the thread's worktree. */
export function WorktreeSetupBar({ threadId }: { threadId: string }) {
  const run = useWorktreeSetupStore((state) => state.runs[threadId])
  const dismissed = useWorktreeSetupStore(
    (state) => state.dismissed[threadId] === true
  )
  const dismiss = useWorktreeSetupStore((state) => state.dismiss)
  const showTerminal = useTerminalStore((state) => state.showTerminal)
  if (!run || dismissed) return null
  const failed = run.status === 'failed'
  const running = run.status === 'running'

  return (
    <div
      className={cn(
        CHAT_LANE,
        'border-border bg-muted/40 mb-2 flex flex-col gap-1.5 rounded-xl border py-1.5 ps-3 pe-1.5 text-xs'
      )}
    >
      <div className='flex items-center gap-2'>
        <HugeiconsIcon
          icon={
            running
              ? Loading03Icon
              : failed
                ? Alert02Icon
                : CheckmarkCircle02Icon
          }
          strokeWidth={2}
          className={cn(
            'size-3.5 shrink-0',
            running && 'text-muted-foreground animate-spin',
            failed && 'text-status-error',
            run.status === 'succeeded' && 'text-pr-open'
          )}
        />
        <span className='min-w-0 flex-1 truncate'>
          {headline(run)}
          {run.heldMessages > 0 ? (
            <span className='text-muted-foreground'>
              {' · '}
              {m.worktree_setup_held({ count: run.heldMessages })}
            </span>
          ) : null}
        </span>
        <Button
          size='xs'
          variant='ghost'
          onClick={() => showTerminal(threadId, run.terminalId)}
        >
          {m.worktree_setup_open_terminal()}
        </Button>
        {failed ? (
          <Button
            size='xs'
            variant='ghost'
            onClick={() =>
              void invoke('rerun_worktree_setup', { threadId }).catch(() => {})
            }
          >
            {m.worktree_setup_run_again()}
          </Button>
        ) : null}
        {failed && run.heldMessages > 0 ? (
          <Button
            size='xs'
            onClick={() =>
              void invoke('skip_worktree_setup', { threadId }).catch(() => {})
            }
          >
            {m.worktree_setup_send_anyway()}
          </Button>
        ) : null}
        {running ? null : (
          <Button
            size='icon-xs'
            variant='ghost'
            aria-label={m.worktree_setup_dismiss()}
            onClick={() => dismiss(threadId)}
          >
            <HugeiconsIcon icon={Cancel01Icon} strokeWidth={2} />
          </Button>
        )}
      </div>
      {running || failed ? (
        run.output.length > 0 ? (
          <pre className='text-muted-foreground overflow-hidden pe-1.5 font-mono text-[0.6875rem] leading-4'>
            {run.output.map((line, index) => (
              <div key={index} className='truncate'>
                {line}
              </div>
            ))}
          </pre>
        ) : null
      ) : null}
    </div>
  )
}
