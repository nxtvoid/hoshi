import { m } from '@/paraglide/messages'
import {
  InformationCircleIcon,
  PaintBoardIcon,
  PlugSocketIcon
} from '@hugeicons/core-free-icons'

export const SETTINGS_SECTIONS = ['appearance', 'providers', 'about'] as const

export type SettingsSection = (typeof SETTINGS_SECTIONS)[number]

export function isSettingsSection(value: string): value is SettingsSection {
  return (SETTINGS_SECTIONS as readonly string[]).includes(value)
}

export const SECTION_META: Record<
  SettingsSection,
  { label: () => string; icon: typeof PaintBoardIcon }
> = {
  appearance: { label: m.settings_appearance, icon: PaintBoardIcon },
  providers: { label: m.settings_providers, icon: PlugSocketIcon },
  about: { label: m.settings_about, icon: InformationCircleIcon }
}

/** Sidebar order. A group without a heading is set apart by space alone. */
export const NAV_GROUPS: {
  heading: (() => string) | null
  sections: SettingsSection[]
}[] = [
  { heading: null, sections: ['appearance'] },
  { heading: m.settings_group_agents, sections: ['providers'] },
  { heading: null, sections: ['about'] }
]

/** Every setting a search can find, with the section that holds it. */
export const SEARCH_ITEMS: { title: () => string; section: SettingsSection }[] =
  [
    { title: m.theme, section: 'appearance' },
    { title: m.claude_code, section: 'providers' },
    { title: m.binary_path, section: 'providers' },
    { title: m.app_version_label, section: 'about' }
  ]

export function searchSettings(query: string) {
  const needle = query.trim().toLowerCase()
  if (!needle) return []
  const sections = SETTINGS_SECTIONS.filter((section) =>
    SECTION_META[section].label().toLowerCase().includes(needle)
  ).map((section) => ({ title: SECTION_META[section].label(), section }))
  const settings = SEARCH_ITEMS.filter((item) =>
    item.title().toLowerCase().includes(needle)
  ).map((item) => ({ title: item.title(), section: item.section }))
  return [...sections, ...settings]
}
