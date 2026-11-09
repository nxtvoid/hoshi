import type { TerminalView } from './TerminalView'
import { type ComponentProps, lazy, Suspense } from 'react'

const View = lazy(() =>
  import('./TerminalView').then((module) => ({ default: module.TerminalView }))
)

/** The terminal, with xterm loaded the first time one shows. */
export function LazyTerminalView(props: ComponentProps<typeof TerminalView>) {
  return (
    <Suspense fallback={null}>
      <View {...props} />
    </Suspense>
  )
}
