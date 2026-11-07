import '@hoshi/ui/globals.css'
import { StrictMode } from 'react'
import { routeTree } from './routeTree.gen'
import { createRoot } from 'react-dom/client'
import { startThemeSync } from './state/theme-store'
import { useSettingsStore } from './state/settings-store'
import { createRouter, RouterProvider } from '@tanstack/react-router'

const router = createRouter({ routeTree })

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router
  }
}

startThemeSync()
useSettingsStore.getState().load()

const root = document.getElementById('root')
if (!root) throw new Error('Missing #root element')

createRoot(root).render(
  <StrictMode>
    <RouterProvider router={router} />
  </StrictMode>
)
