import { useEffect, useRef } from 'react'
import { useAppStore } from '@/state/app-store'
import { firstRunAction } from '@/state/first-run'
import { useSettingsStore } from '@/state/settings-store'
import { useLocation, useNavigate } from '@tanstack/react-router'

/** Opens the setup once for a new user, and marks it done for anyone who already uses hoshi. */
export function FirstRunGate() {
  const navigate = useNavigate()
  const pathname = useLocation({ select: (location) => location.pathname })
  const ready = useAppStore((state) => state.ready)
  const hasProjects = useAppStore((state) => state.projects.length > 0)
  const hasThreads = useAppStore((state) => state.threads.length > 0)
  const hasHistory = useAppStore((state) => state.sequence > 0)
  const completedAt = useSettingsStore(
    (state) => state.settings?.onboardingCompletedAt
  )
  const settled = useRef(false)
  const action = firstRunAction({
    ready,
    completedAt,
    hasProjects,
    hasThreads,
    hasHistory,
    pathname
  })

  useEffect(() => {
    if (settled.current || action === 'wait') return
    settled.current = true
    if (action === 'complete') {
      void useSettingsStore.getState().completeOnboarding()
    } else if (action === 'welcome') {
      void navigate({ to: '/welcome', replace: true })
    }
  }, [action, navigate])

  return null
}
