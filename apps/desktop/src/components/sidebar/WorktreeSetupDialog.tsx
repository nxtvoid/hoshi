import { useId, useState } from 'react'
import { m } from '@/paraglide/messages'
import { invoke } from '@tauri-apps/api/core'
import { Button } from '@hoshi/ui/components/button'
import { Switch } from '@hoshi/ui/components/switch'
import type { Project } from '@/bindings/Project'
import type { Command } from '@/bindings/Command'
import { Textarea } from '@hoshi/ui/components/textarea'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogTitle
} from '@hoshi/ui/components/dialog'

/** Sets the command each new worktree of `project` runs, for this project only. */
export function WorktreeSetupDialog({
  project,
  onClose
}: {
  project: Project
  onClose: () => void
}) {
  const commandId = useId()
  const waitId = useId()
  const [command, setCommand] = useState(project.worktreeSetup?.command ?? '')
  const [wait, setWait] = useState(project.worktreeSetup?.wait ?? false)
  const [saving, setSaving] = useState(false)

  const save = async () => {
    setSaving(true)
    const trimmed = command.trim()
    const update: Command = {
      type: 'projectWorktreeSetupSet',
      projectId: project.id,
      setup: trimmed ? { command: trimmed, wait } : null
    }
    try {
      await invoke('dispatch', { command: update })
      onClose()
    } catch (error) {
      console.error('hoshi: failed to save the worktree setup', error)
      setSaving(false)
    }
  }

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className='sm:max-w-lg'>
        <DialogTitle>{m.worktree_setup_title()}</DialogTitle>
        <DialogDescription>
          {m.worktree_setup_description({ project: project.title })}
        </DialogDescription>
        <div className='flex flex-col gap-1'>
          <label htmlFor={commandId} className='text-muted-foreground text-xs'>
            {m.worktree_setup_command()}
          </label>
          <Textarea
            id={commandId}
            value={command}
            onChange={(event) => setCommand(event.target.value)}
            placeholder={m.worktree_setup_placeholder()}
            className='font-mono text-xs'
            rows={3}
          />
        </div>
        <div className='flex items-start gap-3'>
          <Switch
            id={waitId}
            checked={wait}
            onCheckedChange={setWait}
            className='mt-0.5'
          />
          <label htmlFor={waitId} className='flex flex-col gap-0.5 text-sm'>
            {m.worktree_setup_wait()}
            <span className='text-muted-foreground text-xs'>
              {m.worktree_setup_wait_hint()}
            </span>
          </label>
        </div>
        <DialogFooter>
          <Button variant='outline' onClick={onClose}>
            {m.cancel()}
          </Button>
          <Button disabled={saving} onClick={() => void save()}>
            {m.save()}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
