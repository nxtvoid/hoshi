import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { Button } from '@hoshi/ui/components/button'
import { Textarea } from '@hoshi/ui/components/textarea'
import { ArrowUp02Icon } from '@hugeicons/core-free-icons'

export function Composer() {
  return (
    <form
      className='border-border bg-card mx-auto flex w-full max-w-3xl flex-col gap-1 rounded-xl border p-2'
      onSubmit={(event) => event.preventDefault()}
    >
      <Textarea
        placeholder={m.composer_placeholder()}
        aria-label={m.composer_placeholder()}
        className='max-h-60 min-h-12 resize-none border-0 bg-transparent focus-visible:ring-0 dark:bg-transparent'
      />
      <div className='flex justify-end'>
        <Button
          type='submit'
          size='icon-sm'
          aria-label={m.send_message()}
          className='rounded-full'
          disabled
        >
          <HugeiconsIcon icon={ArrowUp02Icon} strokeWidth={2} />
        </Button>
      </div>
    </form>
  )
}
