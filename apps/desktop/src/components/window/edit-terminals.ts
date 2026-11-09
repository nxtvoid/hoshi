import type { Terminal } from '@xterm/xterm'

const terminals = new Map<Element, Terminal>()

/** Lets the edit menu copy from and paste into the terminal shown in `element`. */
export function registerEditTerminal(element: Element, terminal: Terminal) {
  terminals.set(element, terminal)
  return () => {
    if (terminals.get(element) === terminal) terminals.delete(element)
  }
}

export function terminalAt(target: Element) {
  for (const [element, terminal] of terminals) {
    if (element.contains(target)) return terminal
  }
  return null
}
