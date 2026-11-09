import { createFileRoute } from '@tanstack/react-router'
import { WelcomeWizard } from '@/components/onboarding/WelcomeWizard'

export const Route = createFileRoute('/welcome')({ component: WelcomeWizard })
