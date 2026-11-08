/** The terminals docked below a thread's chat, in tab order. */
export type TerminalDrawer = {
  open: boolean
  terminalIds: string[]
  activeId: string | null
}

export const NO_DRAWER: TerminalDrawer = {
  open: false,
  terminalIds: [],
  activeId: null
}

/** Opening a drawer with no terminal starts one. */
export function toggleDrawer(
  drawer: TerminalDrawer,
  makeId: () => string
): TerminalDrawer {
  if (drawer.open) return { ...drawer, open: false }
  if (drawer.terminalIds.length > 0) return { ...drawer, open: true }
  return addTerminal(drawer, makeId)
}

export function addTerminal(
  drawer: TerminalDrawer,
  makeId: () => string
): TerminalDrawer {
  const id = makeId()
  return { open: true, terminalIds: [...drawer.terminalIds, id], activeId: id }
}

export function activateTerminal(
  drawer: TerminalDrawer,
  id: string
): TerminalDrawer {
  return drawer.terminalIds.includes(id) ? { ...drawer, activeId: id } : drawer
}

/**
 * Closing the active terminal focuses its right neighbour, or the left one at the
 * end; closing the last one hides the drawer.
 */
export function closeTerminal(
  drawer: TerminalDrawer,
  id: string
): TerminalDrawer {
  const index = drawer.terminalIds.indexOf(id)
  if (index === -1) return drawer
  const terminalIds = drawer.terminalIds.filter((other) => other !== id)
  if (terminalIds.length === 0) return NO_DRAWER
  if (drawer.activeId !== id) return { ...drawer, terminalIds }
  const next = terminalIds[index] ?? terminalIds[index - 1] ?? null
  return { ...drawer, terminalIds, activeId: next }
}
