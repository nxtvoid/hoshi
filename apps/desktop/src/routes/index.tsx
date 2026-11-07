import { createFileRoute } from '@tanstack/react-router'

// The chat view always renders in the layout; the URL only adds overlays.
export const Route = createFileRoute('/')({ component: () => null })
