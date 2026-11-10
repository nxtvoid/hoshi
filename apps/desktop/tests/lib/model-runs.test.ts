import { describe, expect, test } from 'bun:test'
import type { ModelInfo } from '../../src/bindings/ModelInfo'
import { modelRun, subagentRun } from '../../src/lib/model-runs'
import type { ProviderModels } from '../../src/bindings/ProviderModels'

const model = (
  slug: string,
  shortName: string,
  efforts: ModelInfo['efforts']
) =>
  ({
    slug,
    name: `Claude ${shortName}`,
    shortName,
    legacy: false,
    isNew: false,
    updateRequired: false,
    efforts,
    defaultEffort: efforts.length > 0 ? 'medium' : null,
    fastMode: false,
    ultracode: false
  }) satisfies ModelInfo

const catalog: ProviderModels = {
  provider: 'claudeCode',
  defaultModel: 'claude-opus-5-5',
  models: [
    model('claude-opus-5-5', 'Opus 5.5', ['low', 'medium', 'high', 'max']),
    model('claude-haiku-4-5', 'Haiku 4.5', [])
  ]
}

describe('modelRun', () => {
  test('names the model by its short name at the effort it supports', () => {
    expect(modelRun(catalog, 'claude-opus-5-5', 'low')).toEqual({
      model: 'Opus 5.5',
      effort: 'low'
    })
    expect(modelRun(catalog, 'claude-opus-5-5', 'xhigh')).toEqual({
      model: 'Opus 5.5',
      effort: 'medium'
    })
    expect(modelRun(catalog, 'claude-haiku-4-5', 'low')).toEqual({
      model: 'Haiku 4.5',
      effort: null
    })
  })

  test('a thread without a model of its own runs on the default one', () => {
    expect(modelRun(catalog, null, null)).toEqual({
      model: 'Opus 5.5',
      effort: 'medium'
    })
    expect(modelRun(null, null, 'low')).toBeNull()
  })

  test('a model the catalog does not know keeps its slug', () => {
    expect(modelRun(catalog, 'claude-custom', 'high')).toEqual({
      model: 'claude-custom',
      effort: 'high'
    })
  })
})

describe('subagentRun', () => {
  test('reads only what a subagent call says it runs on', () => {
    expect(
      subagentRun(catalog, {
        kind: 'subagent',
        model: 'claude-haiku-4-5',
        effort: null
      })
    ).toEqual({ model: 'Haiku 4.5', effort: null })
    expect(subagentRun(catalog, null)).toBeNull()
    expect(
      subagentRun(catalog, {
        kind: 'fileChange',
        path: '/repo/a.txt',
        additions: 1,
        deletions: 0,
        diff: null
      })
    ).toBeNull()
  })
})
