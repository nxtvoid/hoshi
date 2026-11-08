import { createFileRoute } from '@tanstack/react-router'
import { UsageModal } from '@/components/usage/UsageModal'

export const Route = createFileRoute('/usage')({ component: UsageModal })
