import { cn } from 'cn'
import { Copy01Icon } from '@hugeicons/core-free-icons'
import { CopyButton } from '@hoshi/ui/components/copy-button'

type CommandBlockProps = {
  command: string
  copyLabel: string
  copiedLabel: string
  className?: string
}

/** A command to run in a terminal, with a button that copies it. */
export function CommandBlock({
  command,
  copyLabel,
  copiedLabel,
  className
}: CommandBlockProps) {
  return (
    <div
      className={cn(
        'border-border bg-muted/50 flex items-center gap-3 rounded-lg border py-1.5 ps-3 pe-1.5',
        className
      )}
    >
      <code className='min-w-0 flex-1 overflow-x-auto font-mono text-[0.8rem] whitespace-nowrap'>
        {command}
      </code>
      <CopyButton
        value={command}
        label={copyLabel}
        copiedLabel={copiedLabel}
        icon={Copy01Icon}
        side='top'
        className='text-muted-foreground hover:bg-muted hover:text-foreground shrink-0 rounded-md p-1.5'
      />
    </div>
  )
}
