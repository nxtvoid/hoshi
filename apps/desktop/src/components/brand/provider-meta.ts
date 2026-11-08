import { m } from '@/paraglide/messages'
import { ClaudeIcon } from './ProviderIcon'

type ProviderMeta = {
  label: () => string
  icon: typeof ClaudeIcon
  /** Classes for the provider's mark: its color, and a tinted tile behind it. */
  tint: string
  tile: string
  /** The fill of the provider's usage bars. */
  bar: string
}

const PROVIDER_META: Record<string, ProviderMeta> = {
  claudeCode: {
    label: m.claude_code,
    icon: ClaudeIcon,
    tint: 'text-provider-claude',
    tile: 'bg-provider-claude/15 text-provider-claude',
    bar: 'bg-provider-claude/45'
  }
}

const FALLBACK_META = PROVIDER_META.claudeCode

export const metaOf = (provider: string | undefined) =>
  (provider && PROVIDER_META[provider]) || FALLBACK_META
