import { m } from '@/paraglide/messages'

export function App() {
  return (
    <main className='flex h-screen flex-col items-center justify-center gap-2 bg-neutral-950 text-neutral-200'>
      <h1 className='text-2xl font-medium tracking-tight'>hoshi</h1>
      <p className='text-sm text-neutral-500'>{m.no_project_selected()}</p>
    </main>
  )
}
