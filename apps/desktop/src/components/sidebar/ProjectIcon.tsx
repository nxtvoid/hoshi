import * as React from 'react'
import { cn } from '@hoshi/utils'
import { useProjectIconStore } from '@/state/project-icon-store'

export function ProjectIcon({
  projectId,
  className,
  fallback
}: {
  projectId: string
  className?: string
  fallback: React.ReactNode
}) {
  const icon = useProjectIconStore((state) => state.icons[projectId])
  const load = useProjectIconStore((state) => state.load)
  const [failed, setFailed] = React.useState<string | null>(null)

  React.useEffect(() => {
    load(projectId)
  }, [load, projectId])

  if (!icon || failed === icon) return fallback

  return (
    <img
      src={icon}
      alt=''
      draggable={false}
      onError={() => setFailed(icon)}
      className={cn('shrink-0 object-contain', className)}
    />
  )
}
