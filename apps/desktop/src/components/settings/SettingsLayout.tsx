import { cn } from '@hoshi/utils'
import type { ReactNode } from 'react'

export function SettingsPage({ children }: { children: ReactNode }) {
  return (
    <div className='min-h-0 flex-1 overflow-y-auto'>
      <div className='mx-auto flex w-full max-w-4xl flex-col gap-8 px-6 pt-6 pb-12'>
        {children}
      </div>
    </div>
  )
}

type SettingsSectionProps = {
  title: string
  hideTitle?: boolean
  children: ReactNode
}

export function SettingsSection({
  title,
  hideTitle = false,
  children
}: SettingsSectionProps) {
  return (
    <section className={cn(!hideTitle && 'flex flex-col gap-2.5')}>
      <h2
        className={cn(hideTitle ? 'sr-only' : 'px-4 text-base font-semibold')}
      >
        {title}
      </h2>
      <div className='border-border/60 bg-card/40 [&>*+*]:border-border/50 rounded-xl border [&>*+*]:border-t'>
        {children}
      </div>
    </section>
  )
}

type SettingsRowProps = {
  title: ReactNode
  description?: ReactNode
  /** Links the title to the control for screen readers and clicks. */
  htmlFor?: string
  control?: ReactNode
  children?: ReactNode
}

export function SettingsRow({
  title,
  description,
  htmlFor,
  control,
  children
}: SettingsRowProps) {
  return (
    <div className='@container/settings-row px-4 py-3'>
      <div className='flex flex-col gap-3 @min-[32rem]/settings-row:grid @min-[32rem]/settings-row:grid-cols-[minmax(0,1fr)_auto] @min-[32rem]/settings-row:items-center @min-[32rem]/settings-row:gap-8'>
        <div className='flex min-w-0 flex-col gap-1'>
          <label htmlFor={htmlFor} className='text-sm font-medium'>
            {title}
          </label>
          {description ? (
            <p className='text-muted-foreground max-w-2xl text-xs text-pretty'>
              {description}
            </p>
          ) : null}
        </div>
        {control ? (
          <div className='flex min-w-0 shrink-0 items-center gap-2 @min-[32rem]/settings-row:justify-end'>
            {control}
          </div>
        ) : null}
      </div>
      {children}
    </div>
  )
}
