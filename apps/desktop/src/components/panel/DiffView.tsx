import { useMemo } from 'react'
import { cn } from '@hoshi/utils'
import { type DiffLineKind, parseUnifiedDiff } from '@/lib/unified-diff'

const LINE_STYLE: Record<DiffLineKind, string> = {
  hunk: 'bg-muted/60 text-muted-foreground',
  added: 'bg-diff-added',
  removed: 'bg-diff-removed',
  context: '',
  note: 'text-muted-foreground italic'
}

/** One file's unified diff with old and new line numbers. */
export function DiffView({ text }: { text: string }) {
  const lines = useMemo(() => parseUnifiedDiff(text), [text])

  return (
    <div className='overflow-x-auto pb-1 font-mono text-[0.6875rem] leading-5'>
      <table className='w-full min-w-max border-collapse'>
        <tbody>
          {lines.map((line, index) => (
            <tr key={index} className={LINE_STYLE[line.kind]}>
              <td className='text-muted-foreground w-10 pr-1 text-right align-top tabular-nums select-none'>
                {line.oldLine}
              </td>
              <td className='text-muted-foreground w-10 pr-2 text-right align-top tabular-nums select-none'>
                {line.newLine}
              </td>
              <td className='pr-3 whitespace-pre'>
                <span
                  className={cn(
                    'inline-block w-3 select-none',
                    line.kind === 'added' && 'text-pr-open',
                    line.kind === 'removed' && 'text-status-error'
                  )}
                >
                  {line.kind === 'added'
                    ? '+'
                    : line.kind === 'removed'
                      ? '−'
                      : ''}
                </span>
                {line.text}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
