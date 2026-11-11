/** Projects in the order the user dragged them into; projects added since go last, oldest first. */
export function orderProjects<T extends { id: string }>(
  projects: readonly T[],
  order: readonly string[]
): T[] {
  const rank = new Map(order.map((id, index) => [id, index]))
  return projects
    .map((project, index) => ({ project, index }))
    .sort(
      (a, b) =>
        (rank.get(a.project.id) ?? order.length + a.index) -
        (rank.get(b.project.id) ?? order.length + b.index)
    )
    .map(({ project }) => project)
}

/** The ids of `ids` with `activeId` moved to where `overId` was. */
export function moveProject(
  ids: readonly string[],
  activeId: string,
  overId: string
): string[] {
  const from = ids.indexOf(activeId)
  const to = ids.indexOf(overId)
  if (from < 0 || to < 0 || from === to) return [...ids]
  const next = [...ids]
  next.splice(from, 1)
  next.splice(to, 0, activeId)
  return next
}
