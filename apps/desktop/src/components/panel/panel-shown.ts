import { createContext } from 'react'

/** Whether the side panel is on screen; it stays mounted while closed, to open again at once. */
export const PanelShown = createContext(true)
