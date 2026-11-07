import { isSettingsSection } from '@/components/settings/sections'
import { createFileRoute, redirect } from '@tanstack/react-router'
import { SettingsModal } from '@/components/settings/SettingsModal'

export const Route = createFileRoute('/settings/$section')({
  beforeLoad: ({ params }) => {
    if (!isSettingsSection(params.section)) {
      throw redirect({
        to: '/settings/$section',
        params: { section: 'appearance' },
        replace: true
      })
    }
  },
  component: SettingsRoute
})

function SettingsRoute() {
  const { section } = Route.useParams()
  return isSettingsSection(section) ? <SettingsModal section={section} /> : null
}
