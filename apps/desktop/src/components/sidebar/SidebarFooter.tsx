import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { IconButton } from '@hoshi/ui/components/icon-button'
import { useNavigate, useRouterState } from '@tanstack/react-router'
import { BarChartIcon, Settings02Icon } from '@hugeicons/core-free-icons'
import { UpdatesButton } from '@/components/updates/UpdatesButton'

export function SidebarFooter() {
  const navigate = useNavigate()
  const onSettings = useRouterState({
    select: (state) => state.location.pathname.startsWith('/settings')
  })
  const onUsage = useRouterState({
    select: (state) => state.location.pathname === '/usage'
  })

  return (
    <footer className='border-sidebar-border flex items-center gap-0.5 border-t px-2 py-1.5'>
      <IconButton
        label={m.settings()}
        side='top'
        active={onSettings}
        onClick={() =>
          navigate({
            to: '/settings/$section',
            params: { section: 'general' }
          })
        }
      >
        <HugeiconsIcon icon={Settings02Icon} strokeWidth={2} />
      </IconButton>
      <IconButton
        label={m.usage()}
        side='top'
        active={onUsage}
        onClick={() => navigate({ to: '/usage' })}
      >
        <HugeiconsIcon icon={BarChartIcon} strokeWidth={2} />
      </IconButton>
      <div className='ms-auto'>
        <UpdatesButton />
      </div>
    </footer>
  )
}
