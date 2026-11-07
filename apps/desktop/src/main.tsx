import './globals.css'
import { App } from './App'
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { startThemeSync } from './state/theme-store'

startThemeSync()

const root = document.getElementById('root')
if (!root) throw new Error('Missing #root element')

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>
)
