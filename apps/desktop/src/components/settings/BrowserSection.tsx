import { useState } from 'react'
import { m } from '@/paraglide/messages'
import { Input } from '@hoshi/ui/components/input'
import { Switch } from '@hoshi/ui/components/switch'
import type { PageSize } from '@/bindings/PageSize'
import { useSettingsStore } from '@/state/settings-store'
import type { LinkTarget } from '@/bindings/LinkTarget'
import type { SearchEngine } from '@/bindings/SearchEngine'
import type { BrowserSettings } from '@/bindings/BrowserSettings'
import type { PageColorScheme } from '@/bindings/PageColorScheme'
import {
  SettingsAdvanced,
  SettingsPage,
  SettingsRow,
  SettingsSection
} from './SettingsLayout'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue
} from '@hoshi/ui/components/select'

const ZOOMS = [50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200]

type Item<T extends string> = { value: T; label: string }

function Choice<T extends string>({
  id,
  items,
  value,
  onChange
}: {
  id: string
  items: Item<T>[]
  value: T
  onChange: (value: T) => void
}) {
  return (
    <Select
      items={items}
      value={value}
      onValueChange={(next: T | null) => {
        if (next) onChange(next)
      }}
    >
      <SelectTrigger id={id} className='min-w-40'>
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {items.map((item) => (
          <SelectItem key={item.value} value={item.value}>
            {item.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}

/** An http(s) URL with `%s` for the terms, as the engine takes it. */
const usableSearchUrl = (url: string) =>
  /^https?:\/\//.test(url) && url.includes('%s')

function CustomSearchUrl({
  value,
  onCommit
}: {
  value: string
  onCommit: (value: string) => void
}) {
  const [draft, setDraft] = useState(value)
  return (
    <SettingsRow
      title={m.browser_custom_search_url()}
      description={
        usableSearchUrl(value)
          ? m.browser_custom_search_url_description()
          : m.browser_custom_search_url_invalid()
      }
      htmlFor='settings-custom-search-url'
      control={
        <Input
          id='settings-custom-search-url'
          className='w-64'
          value={draft}
          spellCheck={false}
          autoCapitalize='none'
          placeholder='https://example.com/search?q=%s'
          onChange={(event) => setDraft(event.target.value)}
          onBlur={() => {
            if (draft.trim() !== value) onCommit(draft.trim())
          }}
          onKeyDown={(event) => {
            if (event.key === 'Enter') event.currentTarget.blur()
          }}
        />
      }
    />
  )
}

export function BrowserSection() {
  const browser = useSettingsStore((state) => state.settings?.browser)
  const updateSettings = useSettingsStore((state) => state.updateSettings)
  const error = useSettingsStore((state) => state.error)

  if (!browser) {
    return (
      <SettingsPage>
        {error === 'load' ? (
          <p className='text-destructive text-sm'>{m.settings_load_failed()}</p>
        ) : null}
      </SettingsPage>
    )
  }

  const update = (patch: Partial<BrowserSettings>) =>
    updateSettings({ browser: { ...browser, ...patch } })
  const linkTargets: Item<LinkTarget>[] = [
    { value: 'system', label: m.browser_links_system() },
    { value: 'hoshi', label: m.browser_links_hoshi() }
  ]
  const engines: Item<SearchEngine>[] = [
    { value: 'duckDuckGo', label: 'DuckDuckGo' },
    { value: 'google', label: 'Google' },
    { value: 'bing', label: 'Bing' },
    { value: 'brave', label: 'Brave Search' },
    { value: 'kagi', label: 'Kagi' },
    { value: 'ecosia', label: 'Ecosia' },
    { value: 'custom', label: m.browser_search_custom() }
  ]
  const sizes: Item<PageSize>[] = [
    { value: 'fill', label: m.browser_size_fill() },
    { value: 'desktop', label: `${m.browser_size_desktop()} · 1280 × 800` },
    { value: 'tablet', label: `${m.browser_size_tablet()} · 768 × 1024` },
    { value: 'phone', label: `${m.browser_size_phone()} · 390 × 844` }
  ]
  const zooms: Item<string>[] = (
    ZOOMS.includes(browser.zoom)
      ? ZOOMS
      : [...ZOOMS, browser.zoom].sort((a, b) => a - b)
  ).map((zoom) => ({ value: String(zoom), label: `${zoom}%` }))
  const schemes: Item<PageColorScheme>[] = [
    { value: 'system', label: m.theme_system() },
    { value: 'light', label: m.theme_light() },
    { value: 'dark', label: m.theme_dark() }
  ]

  return (
    <SettingsPage>
      <SettingsSection title={m.browser_agent_access_heading()}>
        <SettingsRow
          title={m.browser_agent_access()}
          description={m.browser_agent_access_description()}
          htmlFor='settings-agent-browser-access'
          control={
            <Switch
              id='settings-agent-browser-access'
              checked={browser.agentAccess}
              onCheckedChange={(agentAccess) => update({ agentAccess })}
            />
          }
        />
      </SettingsSection>
      <SettingsSection title={m.browser_browsing_heading()}>
        <SettingsRow
          title={m.browser_open_links_in()}
          description={m.browser_open_links_in_description()}
          htmlFor='settings-open-links-in'
          control={
            <Choice
              id='settings-open-links-in'
              items={linkTargets}
              value={browser.openLinksIn}
              onChange={(openLinksIn) => update({ openLinksIn })}
            />
          }
        />
        <SettingsRow
          title={m.float_agent_pages_setting()}
          description={m.float_agent_pages_setting_description()}
          htmlFor='settings-float-agent-pages'
          control={
            <Switch
              id='settings-float-agent-pages'
              checked={browser.floatAgentPages}
              onCheckedChange={(floatAgentPages) => update({ floatAgentPages })}
            />
          }
        />
        <SettingsRow
          title={m.browser_search_engine()}
          description={m.browser_search_engine_description()}
          htmlFor='settings-search-engine'
          control={
            <Choice
              id='settings-search-engine'
              items={engines}
              value={browser.searchEngine}
              onChange={(searchEngine) => update({ searchEngine })}
            />
          }
        />
        {browser.searchEngine === 'custom' ? (
          <CustomSearchUrl
            value={browser.customSearchUrl}
            onCommit={(customSearchUrl) => update({ customSearchUrl })}
          />
        ) : null}
        <SettingsAdvanced label={m.advanced()}>
          <SettingsRow
            title={m.browser_page_size()}
            description={m.browser_page_size_description()}
            htmlFor='settings-page-size'
            control={
              <Choice
                id='settings-page-size'
                items={sizes}
                value={browser.pageSize}
                onChange={(pageSize) => update({ pageSize })}
              />
            }
          />
          <SettingsRow
            title={m.browser_zoom()}
            description={m.browser_zoom_description()}
            htmlFor='settings-page-zoom'
            control={
              <Choice
                id='settings-page-zoom'
                items={zooms}
                value={String(browser.zoom)}
                onChange={(zoom) => update({ zoom: Number(zoom) })}
              />
            }
          />
          <SettingsRow
            title={m.browser_color_scheme()}
            description={m.browser_color_scheme_description()}
            htmlFor='settings-page-color-scheme'
            control={
              <Choice
                id='settings-page-color-scheme'
                items={schemes}
                value={browser.colorScheme}
                onChange={(colorScheme) => update({ colorScheme })}
              />
            }
          />
        </SettingsAdvanced>
      </SettingsSection>
    </SettingsPage>
  )
}
