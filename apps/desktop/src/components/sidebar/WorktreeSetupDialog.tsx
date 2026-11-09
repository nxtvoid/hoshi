import { m } from '@/paraglide/messages'
import { invoke } from '@tauri-apps/api/core'
import { HugeiconsIcon } from '@hugeicons/react'
import type { Project } from '@/bindings/Project'
import type { Command } from '@/bindings/Command'
import { Button } from '@hoshi/ui/components/button'
import { Switch } from '@hoshi/ui/components/switch'
import { Textarea } from '@hoshi/ui/components/textarea'
import type { CommandError } from '@/bindings/CommandError'
import type { WorktreeSetup } from '@/bindings/WorktreeSetup'
import { parseSetupFiles, shareSetup } from '@/lib/worktree-setup'
import { useEffect, useId, useMemo, useRef, useState } from 'react'
import { Share08Icon, Tick02Icon } from '@hugeicons/core-free-icons'
import { WorktreeSetupImport } from './WorktreeSetupImport'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogTitle
} from '@hoshi/ui/components/dialog'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger
} from '@hoshi/ui/components/dropdown-menu'

const COPIED_MS = 1500

/** Sets how each new worktree of `project` is prepared, for this project only. */
export function WorktreeSetupDialog({
  project,
  onClose
}: {
  project: Project
  onClose: () => void
}) {
  const filesId = useId()
  const commandId = useId()
  const waitId = useId()
  const [filesText, setFilesText] = useState(
    project.worktreeSetup?.files.join('\n') ?? ''
  )
  const [command, setCommand] = useState(project.worktreeSetup?.command ?? '')
  const [wait, setWait] = useState(project.worktreeSetup?.wait ?? false)
  const [saving, setSaving] = useState(false)
  const [refused, setRefused] = useState<string | null>(null)
  const [importing, setImporting] = useState(false)
  const [copied, setCopied] = useState(false)
  const copiedTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const files = useMemo(() => parseSetupFiles(filesText), [filesText])

  useEffect(
    () => () => {
      if (copiedTimer.current) clearTimeout(copiedTimer.current)
    },
    []
  )

  const invalid =
    refused && !files.invalid.includes(refused)
      ? [...files.invalid, refused]
      : files.invalid
  const trimmed = command.trim()
  const setup: WorktreeSetup | null =
    files.files.length > 0 || trimmed
      ? { files: files.files, command: trimmed, wait }
      : null

  const save = async () => {
    setSaving(true)
    const update: Command = {
      type: 'projectWorktreeSetupSet',
      projectId: project.id,
      setup
    }
    try {
      await invoke('dispatch', { command: update })
      onClose()
    } catch (error) {
      const refusal = error as CommandError
      if (refusal.code === 'setupFileInvalid') setRefused(refusal.path)
      else console.error('hoshi: failed to save the worktree setup', error)
      setSaving(false)
    }
  }

  const copySetup = () => {
    if (!setup) return
    void navigator.clipboard.writeText(shareSetup(setup)).then(() => {
      if (copiedTimer.current) clearTimeout(copiedTimer.current)
      setCopied(true)
      copiedTimer.current = setTimeout(() => setCopied(false), COPIED_MS)
    })
  }

  const importSetup = (shared: WorktreeSetup) => {
    setFilesText(shared.files.join('\n'))
    setCommand(shared.command)
    setWait(shared.wait)
    setRefused(null)
    setImporting(false)
  }

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className='sm:max-w-lg'>
        {importing ? (
          <WorktreeSetupImport
            projectTitle={project.title}
            onImport={importSetup}
            onBack={() => setImporting(false)}
          />
        ) : (
          <>
            <DialogTitle>{m.worktree_setup_title()}</DialogTitle>
            <DialogDescription>
              {m.worktree_setup_description({ project: project.title })}
            </DialogDescription>
            <div className='flex flex-col gap-1'>
              <label
                htmlFor={filesId}
                className='text-muted-foreground text-xs'
              >
                {m.worktree_setup_files()}
              </label>
              <Textarea
                id={filesId}
                value={filesText}
                aria-invalid={invalid.length > 0 || undefined}
                onChange={(event) => {
                  setFilesText(event.target.value)
                  setRefused(null)
                }}
                placeholder={m.worktree_setup_files_placeholder()}
                className='font-mono text-xs'
                rows={3}
              />
              {invalid.length > 0 ? (
                <div className='text-destructive flex flex-col text-xs'>
                  {invalid.map((path) => (
                    <span key={path}>
                      {m.worktree_setup_file_invalid({ path })}
                    </span>
                  ))}
                </div>
              ) : null}
              <span className='text-muted-foreground text-xs'>
                {m.worktree_setup_files_hint()}
              </span>
            </div>
            <div className='flex flex-col gap-1'>
              <label
                htmlFor={commandId}
                className='text-muted-foreground text-xs'
              >
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
              <span className='text-muted-foreground text-xs'>
                {m.worktree_setup_command_hint()}
              </span>
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
              <DropdownMenu>
                <DropdownMenuTrigger
                  render={<Button variant='ghost' className='sm:me-auto' />}
                >
                  <HugeiconsIcon
                    icon={copied ? Tick02Icon : Share08Icon}
                    strokeWidth={2}
                  />
                  {copied ? m.copied() : m.worktree_setup_share()}
                </DropdownMenuTrigger>
                <DropdownMenuContent align='start' className='w-44'>
                  <DropdownMenuItem
                    disabled={!setup || invalid.length > 0}
                    onClick={copySetup}
                  >
                    {m.worktree_setup_copy()}
                  </DropdownMenuItem>
                  <DropdownMenuItem onClick={() => setImporting(true)}>
                    {m.worktree_setup_import_menu()}
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
              <Button variant='outline' onClick={onClose}>
                {m.cancel()}
              </Button>
              <Button
                disabled={saving || invalid.length > 0}
                onClick={() => void save()}
              >
                {m.save()}
              </Button>
            </DialogFooter>
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}
