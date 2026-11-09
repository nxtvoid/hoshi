import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { CHIP_CLASSES } from '@/lib/chips'
import { DiffView } from '@/components/panel/DiffView'
import { FileIcon } from '@/components/files/FileIcon'

const PREVIEW_PATCH = [
  '@@ -1,3 +1,3 @@',
  ' export function formatUser(user: User) {',
  '-  return user.name.toUpperCase();',
  '+  return user.name.trim() || user.email; // 0O 1lI',
  ' }'
].join('\n')

function Chip({
  kind,
  children
}: {
  kind: 'file' | 'skill'
  children: string
}) {
  return (
    <span className={cn('rounded-sm px-0.5', CHIP_CLASSES[kind])}>
      {children}
    </span>
  )
}

/** A sent prompt with mentions, in the prompt box's font and size. */
export function PromptFontPreview() {
  return (
    <div className='border-border bg-background mt-3 rounded-lg border px-3 py-2'>
      <p className='font-(family-name:--font-composer) leading-relaxed text-(length:--composer-font-size)'>
        {m.prompt_preview_use()} <Chip kind='skill'>$frontend-design</Chip>{' '}
        {m.prompt_preview_fix()} <Chip kind='file'>surface.test.ts</Chip>{' '}
        {m.prompt_preview_align()} <Chip kind='file'>SettingsPanels.tsx</Chip>{' '}
        {m.prompt_preview_ship()}
      </p>
    </div>
  )
}

/** One file's diff, as the changes panel shows it. */
export function CodeFontPreview() {
  const path = 'src/formatUser.ts'
  return (
    <div className='border-border bg-background mt-3 overflow-hidden rounded-lg border'>
      <div className='border-border flex h-8 items-center gap-2 border-b px-3 text-xs'>
        <FileIcon path={path} className='size-3.5' />
        <span className='min-w-0 flex-1 truncate font-mono'>{path}</span>
        <span className='text-pr-open tabular-nums'>+1</span>
        <span className='text-status-error tabular-nums'>−1</span>
      </div>
      <DiffView text={PREVIEW_PATCH} />
    </div>
  )
}

/** A few lines of a dev server, in the terminal's font and size. */
export function TerminalFontPreview() {
  return (
    <pre className='border-border bg-background mt-3 overflow-hidden rounded-lg border p-3 font-(family-name:--font-terminal) leading-[1.2] text-(length:--terminal-font-size)'>
      <span className='text-status-done font-bold'>→</span>{' '}
      <span className='text-link font-bold'>hoshi</span>{' '}
      <span className='text-file-blue'>git:(</span>
      <span className='text-status-error'>main</span>
      <span className='text-file-blue'>)</span> bun dev
      {'\n\n  '}
      <span className='text-status-done font-bold'>VITE</span>{' '}
      <span className='text-status-done'>v8.0.16</span>{' '}
      <span className='text-muted-foreground'>ready in</span>{' '}
      <span className='font-bold'>1.24s</span>
      {'\n\n  '}
      <span className='text-status-done'>→</span>{' '}
      <span className='text-muted-foreground'>Local:</span>
      {'   '}
      <span className='text-link underline'>http://localhost:1420/</span>
      {'\n  '}
      <span className='text-status-done'>✓ 85 passed</span>
      {'   '}
      <span className='text-status-waiting'>△ 2 warnings</span>
      {'   '}
      <span className='text-status-error'>✗ 0 failed</span>
    </pre>
  )
}
