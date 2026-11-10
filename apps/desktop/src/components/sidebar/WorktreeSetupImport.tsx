import { m } from '@/paraglide/messages'
import { useMemo, useState } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import { Button } from '@hoshi/ui/components/button'
import { Alert02Icon, ArrowLeft01Icon } from '@hugeicons/core-free-icons'
import { Textarea } from '@hoshi/ui/components/textarea'
import type { WorktreeSetup } from '@/bindings/WorktreeSetup'
import { parseSetupFiles, parseSharedSetup } from '@/lib/worktree-setup'
import {
  DialogDescription,
  DialogFooter,
  DialogTitle
} from '@hoshi/ui/components/dialog'

/**
 * The setup dialog's import step: takes a setup someone shared and shows it in full
 * before it fills the form.
 */
export function WorktreeSetupImport({
  projectTitle,
  onImport,
  onBack
}: {
  projectTitle: string
  onImport: (setup: WorktreeSetup) => void
  onBack: () => void
}) {
  const [text, setText] = useState('')
  const shared = useMemo(() => parseSharedSetup(text), [text])
  const invalid = useMemo(
    () => (shared ? parseSetupFiles(shared.files).invalid : []),
    [shared]
  )

  return (
    <>
      <div className='-ms-1.5 flex items-center gap-1'>
        <Button
          size='icon-sm'
          variant='ghost'
          aria-label={m.back()}
          onClick={onBack}
        >
          <HugeiconsIcon icon={ArrowLeft01Icon} strokeWidth={2} />
        </Button>
        <DialogTitle>{m.worktree_setup_import_title()}</DialogTitle>
      </div>
      {shared ? (
        <>
          <div className='border-warning/30 bg-warning-surface flex gap-2.5 rounded-lg border p-3'>
            <HugeiconsIcon
              icon={Alert02Icon}
              strokeWidth={2}
              className='text-warning mt-0.5 size-4 shrink-0'
            />
            <div className='flex flex-col gap-0.5'>
              <p className='text-warning font-medium'>
                {m.worktree_setup_import_warning_title()}
              </p>
              <p className='text-muted-foreground text-xs'>
                {m.worktree_setup_import_warning_body()}
              </p>
            </div>
          </div>
          <PreviewBlock
            label={m.worktree_setup_files()}
            value={shared.files.join('\n')}
          />
          {invalid.length > 0 ? (
            <div className='text-destructive -mt-2 flex flex-col text-xs'>
              {invalid.map((path) => (
                <span key={path}>
                  {m.worktree_setup_file_invalid({ path })}
                </span>
              ))}
            </div>
          ) : null}
          <PreviewBlock
            label={m.worktree_setup_command()}
            value={shared.command}
          />
          <p className='text-muted-foreground text-xs'>
            {m.worktree_setup_import_replaces({ project: projectTitle })}
          </p>
        </>
      ) : (
        <>
          <DialogDescription>
            {m.worktree_setup_import_description()}
          </DialogDescription>
          <div className='flex flex-col gap-1'>
            <Textarea
              autoFocus
              value={text}
              aria-label={m.worktree_setup_import_title()}
              aria-invalid={text.trim() !== '' || undefined}
              onChange={(event) => setText(event.target.value)}
              placeholder={m.worktree_setup_import_placeholder()}
              className='font-mono text-xs'
              rows={4}
            />
            {text.trim() ? (
              <span className='text-destructive text-xs'>
                {m.worktree_setup_import_invalid()}
              </span>
            ) : null}
          </div>
        </>
      )}
      <DialogFooter>
        <Button variant='outline' onClick={onBack}>
          {m.back()}
        </Button>
        <Button
          disabled={!shared || invalid.length > 0}
          onClick={() => shared && onImport(shared)}
        >
          {m.worktree_setup_import()}
        </Button>
      </DialogFooter>
    </>
  )
}

function PreviewBlock({ label, value }: { label: string; value: string }) {
  return (
    <div className='flex flex-col gap-1'>
      <span className='text-muted-foreground text-xs'>{label}</span>
      <pre className='bg-muted/40 max-h-40 overflow-auto rounded-lg border px-2.5 py-2 font-mono text-xs break-all whitespace-pre-wrap'>
        {value || (
          <span className='text-muted-foreground font-sans'>
            {m.worktree_setup_import_none()}
          </span>
        )}
      </pre>
    </div>
  )
}
