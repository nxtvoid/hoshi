import type { DiffScope } from '@/bindings/DiffScope'

/**
 * A view the side panel can show. Each kind adds its own parameters; a new kind is a
 * new member here, its defaults in `newSurface`, and an entry in the panel's registry.
 */
export type Surface =
  | {
      id: string
      kind: 'diff'
      /** `turn` shows what the turn `turnId` changed. */
      scope: DiffScope | 'turn'
      turnId?: string
      /** The file to open and scroll to, as a turn's changes ask. */
      focusPath?: string
    }
  | { id: string; kind: 'pullRequest' }
  /** Its id is also its terminal's id. */
  | { id: string; kind: 'terminal' }

export type SurfaceKind = Surface['kind']

/** Kinds a thread shows at most once; opening another one focuses it. */
const SINGLE_INSTANCE: ReadonlySet<SurfaceKind> = new Set([
  'diff',
  'pullRequest'
])

/** The surfaces a thread has open, in tab order. */
export type ThreadSurfaces = { surfaces: Surface[]; activeId: string | null }

export const NO_SURFACES: ThreadSurfaces = { surfaces: [], activeId: null }

export function newSurface(kind: SurfaceKind, id: string): Surface {
  switch (kind) {
    case 'diff':
      return { id, kind, scope: 'uncommitted' }
    case 'pullRequest':
    case 'terminal':
      return { id, kind }
  }
}

/** Focuses the surface of that kind if it is single-instance and open, or adds one. */
export function openSurface(
  panel: ThreadSurfaces,
  kind: SurfaceKind,
  makeId: () => string
): ThreadSurfaces {
  if (SINGLE_INSTANCE.has(kind)) {
    const existing = panel.surfaces.find((surface) => surface.kind === kind)
    if (existing) return { ...panel, activeId: existing.id }
  }
  const surface = newSurface(kind, makeId())
  return { surfaces: [...panel.surfaces, surface], activeId: surface.id }
}

/** Opens the diff surface on what a turn changed, at `focusPath` when given. */
export function showTurnDiff(
  panel: ThreadSurfaces,
  turnId: string,
  focusPath: string | undefined,
  makeId: () => string
): ThreadSurfaces {
  const opened = openSurface(panel, 'diff', makeId)
  return updateSurface(opened, opened.activeId ?? '', (surface) =>
    surface.kind === 'diff'
      ? { ...surface, scope: 'turn', turnId, focusPath }
      : surface
  )
}

export function activateSurface(
  panel: ThreadSurfaces,
  id: string
): ThreadSurfaces {
  return panel.surfaces.some((surface) => surface.id === id)
    ? { ...panel, activeId: id }
    : panel
}

/** Closing the active surface focuses its right neighbour, or the left one at the end. */
export function closeSurface(
  panel: ThreadSurfaces,
  id: string
): ThreadSurfaces {
  const index = panel.surfaces.findIndex((surface) => surface.id === id)
  if (index === -1) return panel
  const surfaces = panel.surfaces.filter((surface) => surface.id !== id)
  if (panel.activeId !== id) return { surfaces, activeId: panel.activeId }
  const next = surfaces[index] ?? surfaces[index - 1] ?? null
  return { surfaces, activeId: next?.id ?? null }
}

export function updateSurface<S extends Surface>(
  panel: ThreadSurfaces,
  id: string,
  change: (surface: S) => S
): ThreadSurfaces {
  return {
    ...panel,
    surfaces: panel.surfaces.map((surface) =>
      surface.id === id ? change(surface as S) : surface
    )
  }
}
