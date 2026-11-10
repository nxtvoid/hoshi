import type { Effort } from '@/bindings/Effort'
import type { ModelInfo } from '@/bindings/ModelInfo'
import type { ToolDetail } from '@/bindings/ToolDetail'
import type { ProviderModels } from '@/bindings/ProviderModels'

/** A model as the UI names it, and the effort it runs at; `null` for none. */
export type ModelRun = { model: string; effort: Effort | null }

export const effectiveEffort = (
  model: ModelInfo | undefined,
  effort: Effort | null
) =>
  effort && model?.efforts.includes(effort)
    ? effort
    : (model?.defaultEffort ?? null)

/**
 * What `slug` runs as at `effort`: the catalog's short name, and the effort it supports.
 * `null` slug is the catalog's default model. A model the catalog doesn't know keeps its
 * slug and effort.
 */
export function modelRun(
  catalog: ProviderModels | null,
  slug: string | null,
  effort: Effort | null
): ModelRun | null {
  const id = slug ?? catalog?.defaultModel ?? null
  if (!id) return null
  const model = catalog?.models.find((candidate) => candidate.slug === id)
  if (!model) return { model: id, effort }
  return { model: model.shortName, effort: effectiveEffort(model, effort) }
}

/** What a subagent's call says it runs on, once known. */
export function subagentRun(
  catalog: ProviderModels | null,
  detail: ToolDetail | null
): ModelRun | null {
  if (detail?.kind !== 'subagent') return null
  return modelRun(catalog, detail.model, detail.effort)
}
