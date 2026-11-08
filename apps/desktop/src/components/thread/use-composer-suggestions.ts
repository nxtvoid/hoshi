import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { m } from '@/paraglide/messages'
import { useAppStore } from '@/state/app-store'
import { useInChat, useScopeProjectId } from '@/state/use-scope'
import { rankCommands } from '@/lib/slash-commands'
import { useSettingsStore } from '@/state/settings-store'
import type { PullRequest } from '@/bindings/PullRequest'
import type { CommandError } from '@/bindings/CommandError'
import type { SlashCommand } from '@/bindings/SlashCommand'
import type { ComposerTrigger } from '@/lib/composer-trigger'
import type { WorkspaceEntry } from '@/bindings/WorkspaceEntry'
import {
  slashCommandsKey,
  useSlashCommandsStore
} from '@/state/slash-commands-store'
import { useAccess } from './ComposerMenus'

const PATH_SEARCH_DELAY_MS = 50
const PULL_REQUEST_SEARCH_DELAY_MS = 200
const NO_COMMANDS: SlashCommand[] = []

/** Commands hoshi runs itself instead of sending them to the agent. */
export type ComposerAction = 'model' | 'plan' | 'default'

export type Suggestion =
  | { id: string; type: 'path'; entry: WorkspaceEntry }
  | {
      id: string
      type: 'command'
      command: SlashCommand
      action: ComposerAction | null
    }
  | { id: string; type: 'pullRequest'; pullRequest: PullRequest }
  | { id: string; type: 'skill'; command: SlashCommand }

type CommandSuggestion = Extract<Suggestion, { type: 'command' }>

const ACTION_DESCRIPTIONS: Record<ComposerAction, () => string> = {
  model: m.composer_command_model,
  plan: m.composer_command_plan,
  default: m.composer_command_default
}

const actionSuggestion = (action: ComposerAction): CommandSuggestion => ({
  id: `action:${action}`,
  type: 'command',
  action,
  command: {
    name: action,
    description: ACTION_DESCRIPTIONS[action](),
    argumentHint: null,
    aliases: [],
    custom: false
  }
})

type Search<T> = { key: string; query: string; found: T[] | 'unavailable' }

/**
 * Runs `search` for `query` once it stops changing for `delay` ms, keeping the
 * last answer for `key` while the next one is on its way.
 */
function useSearch<T>(
  key: string | null,
  query: string | null,
  delay: number,
  search: (query: string) => Promise<T[]>
) {
  const [result, setResult] = useState<Search<T> | null>(null)
  useEffect(() => {
    if (query === null || key === null) return
    let cancelled = false
    const timer = setTimeout(() => {
      search(query)
        .then((found) => {
          if (!cancelled) setResult({ key, query, found })
        })
        .catch((error: CommandError) => {
          if (!cancelled) setResult({ key, query, found: 'unavailable' })
          if (error.code !== 'forgeUnavailable') {
            console.error('hoshi: composer search failed', error)
          }
        })
    }, delay)
    return () => {
      cancelled = true
      clearTimeout(timer)
    }
  }, [key, query, delay])
  return result?.key === key ? result : null
}

/** What to offer for `trigger`: files, commands, pull requests or skills. */
export function useComposerSuggestions(trigger: ComposerTrigger | null) {
  const projectId = useScopeProjectId()
  const chat = useInChat()
  const threadId = useAppStore((state) => state.selectedThreadId)
  const planModeEnabled = useSettingsStore(
    (state) => state.settings?.planModeEnabled ?? false
  )
  const [access] = useAccess()
  const key = projectId || chat ? slashCommandsKey(projectId, threadId) : null

  const actions: ComposerAction[] = [
    'model',
    ...(planModeEnabled && !access.planMode ? (['plan'] as const) : []),
    ...(access.planMode ? (['default'] as const) : [])
  ]

  const paths = useSearch(
    key,
    trigger?.kind === 'path' ? trigger.query : null,
    PATH_SEARCH_DELAY_MS,
    (query) =>
      invoke<WorkspaceEntry[]>('search_paths', { projectId, threadId, query })
  )
  const pullRequests = useSearch(
    projectId,
    trigger?.kind === 'pullRequest' ? trigger.query : null,
    PULL_REQUEST_SEARCH_DELAY_MS,
    (query) =>
      invoke<PullRequest[]>('search_pull_requests', { projectId, query })
  )

  const commands = useSlashCommandsStore((state) =>
    key ? state.byKey[key] : undefined
  )
  const commandsLoading = useSlashCommandsStore((state) =>
    key ? (state.loading[key] ?? false) : false
  )
  const loadCommands = useSlashCommandsStore((state) => state.load)
  const commandsWanted =
    trigger?.kind === 'command' || trigger?.kind === 'skill'
  useEffect(() => {
    if (commandsWanted && key) void loadCommands(projectId, threadId)
  }, [commandsWanted, key, projectId, threadId, loadCommands])

  if (!trigger) return { items: [], status: null, actions }

  const searched = <T>(
    search: Search<T> | null,
    toItem: (found: T) => Suggestion,
    messages: { searching: string; none: string; unavailable?: string }
  ) => {
    const found = search?.found
    const items = Array.isArray(found) ? found.map(toItem) : []
    const status =
      items.length > 0
        ? null
        : found === 'unavailable'
          ? (messages.unavailable ?? messages.none)
          : search?.query !== trigger.query
            ? messages.searching
            : messages.none
    return { items, status, actions }
  }

  switch (trigger.kind) {
    case 'path':
      return searched(
        paths,
        (entry) => ({
          id: `path:${entry.kind}:${entry.path}`,
          type: 'path',
          entry
        }),
        {
          searching: m.composer_suggestions_searching_files(),
          none: m.composer_suggestions_no_files()
        }
      )
    case 'pullRequest':
      return searched(
        pullRequests,
        (pullRequest) => ({
          id: `pullRequest:${pullRequest.number}`,
          type: 'pullRequest',
          pullRequest
        }),
        {
          searching: m.composer_suggestions_searching_pull_requests(),
          none: m.composer_suggestions_no_pull_requests(),
          unavailable: m.composer_suggestions_pull_requests_unavailable()
        }
      )
    case 'skill': {
      const items: Suggestion[] = rankCommands(
        (commands ?? NO_COMMANDS).filter((command) => command.custom),
        trigger.query,
        (command) => command
      ).map((command) => ({
        id: `skill:${command.name}`,
        type: 'skill',
        command
      }))
      const status =
        commands === undefined && commandsLoading
          ? m.composer_suggestions_loading_commands()
          : items.length === 0
            ? m.composer_suggestions_no_skills()
            : null
      return { items, status, actions }
    }
    case 'command': {
      const providerItems: CommandSuggestion[] = (commands ?? NO_COMMANDS)
        .filter((command) => !actions.some((action) => action === command.name))
        .map((command) => ({
          id: `command:${command.name}`,
          type: 'command',
          action: null,
          command
        }))
      const items: Suggestion[] = rankCommands(
        [...actions.map(actionSuggestion), ...providerItems],
        trigger.query,
        (item) => item.command
      )
      const status =
        commands === undefined && commandsLoading
          ? m.composer_suggestions_loading_commands()
          : items.length === 0
            ? m.composer_suggestions_no_commands()
            : null
      return { items, status, actions }
    }
  }
}
