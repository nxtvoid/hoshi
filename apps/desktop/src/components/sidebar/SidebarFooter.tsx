import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { IconButton } from '@hoshi/ui/components/icon-button'
import { useNavigate, useRouterState } from '@tanstack/react-router'
import {
  BarChartIcon,
  GitPullRequestIcon,
  Settings02Icon
} from '@hugeicons/core-free-icons'

export function SidebarFooter() {
  const navigate = useNavigate()
  const onSettings = useRouterState({
    select: (state) => state.location.pathname.startsWith('/settings')
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
            params: { section: 'appearance' }
          })
        }
      >
        <HugeiconsIcon icon={Settings02Icon} strokeWidth={2} />
      </IconButton>
      <IconButton label={m.pull_requests()} side='top' disabled>
        <HugeiconsIcon icon={GitPullRequestIcon} strokeWidth={2} />
      </IconButton>
      <IconButton label={m.usage()} side='top' disabled>
        <HugeiconsIcon icon={BarChartIcon} strokeWidth={2} />
      </IconButton>
    </footer>
  )
}
