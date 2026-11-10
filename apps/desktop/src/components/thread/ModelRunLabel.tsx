import { m } from '@/paraglide/messages'
import { EFFORT_LABELS } from './effort-labels'
import type { ModelRun } from '@/lib/model-runs'

/** The model something runs on and its effort, as muted text beside its title. */
export function ModelRunLabel({ run }: { run: ModelRun | null }) {
  if (!run) return null
  return (
    <span className='text-muted-foreground shrink-0 text-xs'>
      {run.effort
        ? m.model_run({ model: run.model, effort: EFFORT_LABELS[run.effort]() })
        : run.model}
    </span>
  )
}
