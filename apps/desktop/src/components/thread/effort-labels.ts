import { m } from '@/paraglide/messages'
import type { Effort } from '@/bindings/Effort'

export const EFFORT_LABELS: Record<Effort, () => string> = {
  low: m.effort_low,
  medium: m.effort_medium,
  high: m.effort_high,
  xhigh: m.effort_xhigh,
  max: m.effort_max
}

export const EFFORT_SHORT_LABELS: Record<Effort, () => string> = {
  low: m.effort_low,
  medium: m.effort_medium_short,
  high: m.effort_high,
  xhigh: m.effort_xhigh_short,
  max: m.effort_max
}
