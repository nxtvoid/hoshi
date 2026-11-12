import { m } from '@/paraglide/messages'
import type { ComponentType } from 'react'
import { SettingsNav } from './SettingsNav'
import { AboutSection } from './AboutSection'
import { GeneralSection } from './GeneralSection'
import { BrowserSection } from './BrowserSection'
import { HugeiconsIcon } from '@hugeicons/react'
import { useNavigate } from '@tanstack/react-router'
import { ProvidersSection } from './ProvidersSection'
import { SourceControlSection } from './SourceControlSection'
import { AppearanceSection } from './AppearanceSection'
import { ThemeEditorHost } from './ThemeEditorPanel'
import { ArrowRight01Icon } from '@hugeicons/core-free-icons'
import { SECTION_META, type SettingsSection } from './sections'
import { PageModal, PageModalClose } from '@hoshi/ui/components/page-modal'

const CONTENT: Record<SettingsSection, ComponentType> = {
  general: GeneralSection,
  appearance: AppearanceSection,
  providers: ProvidersSection,
  browser: BrowserSection,
  forges: SourceControlSection,
  about: AboutSection
}

export function SettingsModal({ section }: { section: SettingsSection }) {
  const navigate = useNavigate()
  const Content = CONTENT[section]

  return (
    <PageModal
      label={m.settings()}
      topBar={<div data-tauri-drag-region className='h-10' />}
      onClose={() => navigate({ to: '/' })}
    >
      <div className='flex min-h-0 flex-1'>
        <SettingsNav section={section} />
        <div className='flex min-h-0 min-w-0 flex-1 flex-col'>
          <header className='border-border flex h-12 shrink-0 items-center gap-3 border-b ps-5 pe-2'>
            <nav
              aria-label={m.settings()}
              className='flex min-w-0 items-center gap-1.5 text-sm'
            >
              <span className='text-muted-foreground'>{m.settings()}</span>
              <HugeiconsIcon
                icon={ArrowRight01Icon}
                strokeWidth={2}
                className='text-muted-foreground size-3.5'
              />
              <span aria-current='page' className='truncate font-medium'>
                {SECTION_META[section].label()}
              </span>
            </nav>
            <div className='ms-auto'>
              <PageModalClose label={m.close_settings()} />
            </div>
          </header>
          <Content />
        </div>
      </div>
      <ThemeEditorHost />
    </PageModal>
  )
}
