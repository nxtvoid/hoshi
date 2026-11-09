import type { ReactNode } from 'react'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { Switch } from '@hoshi/ui/components/switch'
import { Button } from '@hoshi/ui/components/button'
import { useThemeStore } from '@/state/theme-store'
import { RotateLeft01Icon } from '@hugeicons/core-free-icons'
import {
  AddThemeButton,
  ColorSchemeTiles,
  CreateThemeButton,
  ThemeGrid
} from './ThemeLibrary'
import {
  CodeFontPreview,
  PromptFontPreview,
  TerminalFontPreview
} from './TypographyPreviews'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@hoshi/ui/components/tooltip'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue
} from '@hoshi/ui/components/select'
import {
  type Appearance,
  CODE_SIZE,
  COMPOSER_SIZE,
  DEFAULT_APPEARANCE,
  INTERFACE_SIZE,
  type SizeRange,
  sizeOptions,
  TERMINAL_SIZE
} from '@/state/appearance'
import {
  SettingsAdvanced,
  SettingsPage,
  SettingsRow,
  SettingsSection
} from './SettingsLayout'

type FontOption<T extends string> = { value: T; label: () => string }

const INTERFACE_FONT_OPTIONS: FontOption<Appearance['interfaceFont']>[] = [
  { value: 'geist', label: m.font_geist },
  { value: 'inter', label: m.font_inter },
  { value: 'system', label: m.font_system }
]

const CODE_FONT_OPTIONS: FontOption<Appearance['codeFont']>[] = [
  { value: 'jetbrains', label: m.font_jetbrains_mono },
  { value: 'cascadia', label: m.font_cascadia_mono },
  { value: 'system', label: m.font_system_mono }
]

const COMPOSER_FONT_OPTIONS: FontOption<Appearance['composerFont']>[] = [
  { value: 'interface', label: m.font_same_as_interface },
  ...INTERFACE_FONT_OPTIONS,
  { value: 'jetbrains', label: m.font_jetbrains_mono },
  { value: 'cascadia', label: m.font_cascadia_mono },
  { value: 'system-mono', label: m.font_system_mono }
]

const PLATFORM = document.documentElement.dataset.platform

export function AppearanceSection() {
  return (
    <SettingsPage>
      <section className='flex flex-col gap-2.5'>
        <h2 className='px-4 text-base font-semibold'>{m.color_scheme()}</h2>
        <ColorSchemeTiles />
      </section>
      <section className='flex flex-col gap-2.5'>
        <div className='flex min-h-7 flex-wrap items-center justify-between gap-3 px-4'>
          <h2 className='text-base font-semibold'>{m.themes()}</h2>
          <div className='flex flex-wrap items-center justify-end gap-2'>
            <CreateThemeButton />
            <AddThemeButton />
          </div>
        </div>
        <ThemeGrid />
      </section>
      <TypographySection />
    </SettingsPage>
  )
}

type FontKey = 'interfaceFont' | 'codeFont' | 'composerFont' | 'terminalFont'
type SizeKey = 'interfaceSize' | 'codeSize' | 'composerSize' | 'terminalSize'

function TypographySection() {
  const separate = useThemeStore((state) => state.separateFonts)
  const wordWrap = useThemeStore((state) => state.wordWrap)
  const fontSmoothing = useThemeStore((state) => state.fontSmoothing)
  const lighterText = useThemeStore((state) => state.lighterText)
  const update = useThemeStore((state) => state.update)

  return (
    <SettingsSection title={m.typography()}>
      <FontRow
        id='settings-interface-font'
        title={m.interface_font()}
        description={m.interface_font_description()}
        fontKey='interfaceFont'
        sizeKey='interfaceSize'
        options={INTERFACE_FONT_OPTIONS}
        range={INTERFACE_SIZE}
        sizeLabel={m.interface_font_size()}
        preview={separate ? undefined : <PromptFontPreview />}
      />
      {separate ? (
        <FontRow
          id='settings-prompt-font'
          title={m.prompt_font()}
          description={m.prompt_font_description()}
          fontKey='composerFont'
          sizeKey='composerSize'
          options={COMPOSER_FONT_OPTIONS}
          range={COMPOSER_SIZE}
          sizeLabel={m.prompt_font_size()}
          preview={<PromptFontPreview />}
        />
      ) : null}
      <FontRow
        id='settings-code-font'
        title={separate ? m.code_font() : m.monospace_font()}
        description={
          separate ? m.code_font_description() : m.monospace_font_description()
        }
        fontKey='codeFont'
        sizeKey='codeSize'
        options={CODE_FONT_OPTIONS}
        range={CODE_SIZE}
        sizeLabel={m.code_font_size()}
        preview={
          <>
            <CodeFontPreview />
            {separate ? null : <TerminalFontPreview />}
          </>
        }
      />
      {separate ? (
        <FontRow
          id='settings-terminal-font'
          title={m.terminal_font()}
          description={m.terminal_font_description()}
          fontKey='terminalFont'
          sizeKey='terminalSize'
          options={CODE_FONT_OPTIONS}
          range={TERMINAL_SIZE}
          sizeLabel={m.terminal_font_size()}
          preview={<TerminalFontPreview />}
        />
      ) : null}
      <SettingsRow
        title={m.word_wrap()}
        description={m.word_wrap_description()}
        htmlFor='settings-word-wrap'
        action={
          wordWrap === DEFAULT_APPEARANCE.wordWrap ? null : (
            <ResetButton
              label={m.reset_setting({ name: m.word_wrap() })}
              onClick={() => update({ wordWrap: DEFAULT_APPEARANCE.wordWrap })}
            />
          )
        }
        control={
          <Switch
            id='settings-word-wrap'
            checked={wordWrap}
            onCheckedChange={(checked) => update({ wordWrap: checked })}
          />
        }
      />
      <SettingsAdvanced label={m.advanced()}>
        <SettingsRow
          title={m.separate_fonts()}
          description={m.separate_fonts_description()}
          htmlFor='settings-separate-fonts'
          control={
            <Switch
              id='settings-separate-fonts'
              checked={separate}
              onCheckedChange={(checked) => update({ separateFonts: checked })}
            />
          }
        />
        {PLATFORM === 'mac' ? (
          <SettingsRow
            title={m.font_smoothing()}
            description={m.font_smoothing_description()}
            htmlFor='settings-font-smoothing'
            action={
              fontSmoothing === DEFAULT_APPEARANCE.fontSmoothing ? null : (
                <ResetButton
                  label={m.reset_setting({ name: m.font_smoothing() })}
                  onClick={() =>
                    update({ fontSmoothing: DEFAULT_APPEARANCE.fontSmoothing })
                  }
                />
              )
            }
            control={
              <Switch
                id='settings-font-smoothing'
                checked={fontSmoothing}
                onCheckedChange={(checked) =>
                  update({ fontSmoothing: checked })
                }
              />
            }
          />
        ) : null}
        {PLATFORM === 'linux' ? (
          <SettingsRow
            title={m.lighter_text()}
            description={m.lighter_text_description()}
            htmlFor='settings-lighter-text'
            control={
              <Switch
                id='settings-lighter-text'
                checked={lighterText}
                onCheckedChange={(checked) => update({ lighterText: checked })}
              />
            }
          />
        ) : null}
      </SettingsAdvanced>
    </SettingsSection>
  )
}

function FontRow<T extends string>({
  id,
  title,
  description,
  fontKey,
  sizeKey,
  options,
  range,
  sizeLabel,
  preview
}: {
  id: string
  title: string
  description: string
  fontKey: FontKey
  sizeKey: SizeKey
  options: FontOption<T>[]
  range: SizeRange
  sizeLabel: string
  preview?: ReactNode
}) {
  const font = useThemeStore((state) => state[fontKey]) as T
  const size = useThemeStore((state) => state[sizeKey])
  const update = useThemeStore((state) => state.update)
  const changed =
    font !== DEFAULT_APPEARANCE[fontKey] || size !== DEFAULT_APPEARANCE[sizeKey]

  return (
    <SettingsRow
      title={title}
      description={description}
      htmlFor={id}
      action={
        changed ? (
          <ResetButton
            label={m.reset_setting({ name: title })}
            onClick={() =>
              update({
                [fontKey]: DEFAULT_APPEARANCE[fontKey],
                [sizeKey]: DEFAULT_APPEARANCE[sizeKey]
              })
            }
          />
        ) : null
      }
      control={
        <div className='flex flex-wrap items-center gap-2 @min-[32rem]/settings-row:flex-nowrap'>
          <OptionSelect
            id={id}
            options={options}
            value={font}
            onChange={(value) => update({ [fontKey]: value })}
          />
          <SizeSelect
            label={sizeLabel}
            range={range}
            value={size}
            onChange={(value) => update({ [sizeKey]: value })}
          />
        </div>
      }
    >
      {preview}
    </SettingsRow>
  )
}

function ResetButton({
  label,
  onClick
}: {
  label: string
  onClick: () => void
}) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            variant='ghost'
            size='icon-xs'
            aria-label={label}
            onClick={onClick}
            className='text-muted-foreground size-5'
          />
        }
      >
        <HugeiconsIcon icon={RotateLeft01Icon} strokeWidth={2} />
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  )
}

function OptionSelect<T extends string>({
  id,
  options,
  value,
  onChange
}: {
  id: string
  options: readonly FontOption<T>[]
  value: T
  onChange: (value: T) => void
}) {
  const items = options.map((option) => ({
    value: option.value,
    label: option.label()
  }))
  return (
    <Select
      items={items}
      value={value}
      onValueChange={(next: T | null) => {
        if (next) onChange(next)
      }}
    >
      <SelectTrigger id={id} className='min-w-44'>
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

function SizeSelect({
  label,
  range,
  value,
  onChange
}: {
  label: string
  range: SizeRange
  value: number
  onChange: (value: number) => void
}) {
  const items = sizeOptions(range).map((size) => ({
    value: size,
    label: m.font_size_px({ size })
  }))
  return (
    <Select
      items={items}
      value={value}
      onValueChange={(next: number | null) => {
        if (next !== null && next !== value) onChange(next)
      }}
    >
      <SelectTrigger aria-label={label} className='w-24 shrink-0'>
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
