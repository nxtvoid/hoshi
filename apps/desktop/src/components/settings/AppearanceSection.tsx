import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { Slider } from '@hoshi/ui/components/slider'
import { Switch } from '@hoshi/ui/components/switch'
import { SettingsPage, SettingsRow, SettingsSection } from './SettingsLayout'
import { ComputerIcon, Moon02Icon, Sun03Icon } from '@hugeicons/core-free-icons'
import {
  type Appearance,
  CODE_SIZE,
  type ColorPalette,
  INTERFACE_SIZE,
  type SizeRange,
  type ThemePreference
} from '@/state/appearance'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue
} from '@hoshi/ui/components/select'
import { useResolvedDark, useThemeStore } from '@/state/theme-store'

export const THEME_OPTIONS = [
  { value: 'system', label: m.theme_system, icon: ComputerIcon },
  { value: 'light', label: m.theme_light, icon: Sun03Icon },
  { value: 'dark', label: m.theme_dark, icon: Moon02Icon }
] as const

const PALETTE_OPTIONS = [
  { value: 'hoshi', label: m.palette_hoshi },
  { value: 'grove', label: m.palette_grove },
  { value: 'ocean', label: m.palette_ocean },
  { value: 'ember', label: m.palette_ember },
  { value: 'iris', label: m.palette_iris }
] as const

const INTERFACE_FONT_OPTIONS = [
  { value: 'geist', label: m.font_geist },
  { value: 'inter', label: m.font_inter },
  { value: 'system', label: m.font_system }
] as const

const CODE_FONT_OPTIONS = [
  { value: 'jetbrains', label: m.font_jetbrains_mono },
  { value: 'cascadia', label: m.font_cascadia_mono },
  { value: 'system', label: m.font_system_mono }
] as const

const IS_LINUX = document.documentElement.dataset.platform === 'linux'

const paletteAttribute = (palette: ColorPalette) =>
  palette === 'hoshi' ? undefined : palette

export function AppearanceSection() {
  return (
    <SettingsPage>
      <SettingsSection title={m.settings_appearance()} hideTitle>
        <SettingsRow title={m.theme()} description={m.theme_description()}>
          <ThemeCards />
        </SettingsRow>
        <SettingsRow
          title={m.color_palette()}
          description={m.color_palette_description()}
        >
          <PaletteSwatches />
        </SettingsRow>
      </SettingsSection>
      <TypographySection />
    </SettingsPage>
  )
}

function ThemeCards() {
  const preference = useThemeStore((state) => state.preference)
  const palette = useThemeStore((state) => state.palette)
  const setPreference = useThemeStore((state) => state.setPreference)

  return (
    <fieldset className='mt-3 grid grid-cols-3 gap-3'>
      <legend className='sr-only'>{m.theme()}</legend>
      {THEME_OPTIONS.map(({ value, label, icon }) => {
        const selected = preference === value
        return (
          <button
            key={value}
            type='button'
            aria-pressed={selected}
            onClick={() => setPreference(value)}
            className={cn(
              'focus-visible:ring-ring flex flex-col gap-2 rounded-xl border p-2 text-sm font-medium transition-colors outline-none focus-visible:ring-2',
              selected
                ? 'border-primary ring-primary text-foreground ring-1'
                : 'text-muted-foreground hover:bg-muted/50 hover:text-foreground'
            )}
          >
            <ThemePreview preference={value} palette={palette} />
            <span className='flex items-center gap-1.5 px-1'>
              <HugeiconsIcon icon={icon} strokeWidth={2} className='size-4' />
              {label()}
            </span>
          </button>
        )
      })}
    </fieldset>
  )
}

function ThemePreview({
  preference,
  palette
}: {
  preference: ThemePreference
  palette: ColorPalette
}) {
  if (preference !== 'system') {
    return <PreviewSurface mode={preference} palette={palette} />
  }
  return (
    <div className='relative'>
      <PreviewSurface mode='light' palette={palette} />
      <PreviewSurface
        mode='dark'
        palette={palette}
        className='absolute inset-0 [clip-path:inset(0_0_0_50%)]'
      />
    </div>
  )
}

function PreviewSurface({
  mode,
  palette,
  className
}: {
  mode: 'light' | 'dark'
  palette: ColorPalette
  className?: string
}) {
  return (
    <div
      aria-hidden
      data-palette={paletteAttribute(palette)}
      className={cn(
        mode,
        'bg-background border-border flex h-16 overflow-hidden rounded-md border',
        className
      )}
    >
      <div className='bg-sidebar border-sidebar-border flex w-1/4 flex-col gap-1 border-e p-1.5'>
        <span className='bg-sidebar-foreground/25 h-1 rounded-full' />
        <span className='bg-sidebar-foreground/15 h-1 w-2/3 rounded-full' />
      </div>
      <div className='flex flex-1 flex-col justify-end gap-1.5 p-2'>
        <span className='bg-foreground/15 h-1.5 w-3/4 rounded-full' />
        <span className='bg-primary h-1.5 w-1/2 rounded-full' />
      </div>
    </div>
  )
}

function PaletteSwatches() {
  const palette = useThemeStore((state) => state.palette)
  const update = useThemeStore((state) => state.update)
  const dark = useResolvedDark()

  return (
    <fieldset className='mt-3 flex flex-wrap gap-1'>
      <legend className='sr-only'>{m.color_palette()}</legend>
      {PALETTE_OPTIONS.map(({ value, label }) => {
        const selected = palette === value
        return (
          <button
            key={value}
            type='button'
            aria-pressed={selected}
            onClick={() => update({ palette: value })}
            className={cn(
              'group focus-visible:ring-ring flex w-16 flex-col items-center gap-1.5 rounded-lg py-1.5 text-xs transition-colors outline-none focus-visible:ring-2',
              selected
                ? 'text-foreground'
                : 'text-muted-foreground hover:text-foreground'
            )}
          >
            <span
              className={cn(
                'ring-offset-background flex size-8 items-center justify-center rounded-full ring-offset-2 transition-shadow',
                selected
                  ? 'ring-primary ring-2'
                  : 'ring-border group-hover:ring-foreground/30 ring-1'
              )}
            >
              <span
                data-palette={paletteAttribute(value)}
                className={cn(
                  dark ? 'dark' : 'light',
                  'bg-primary size-7 rounded-full'
                )}
              />
            </span>
            {label()}
          </button>
        )
      })}
    </fieldset>
  )
}

function TypographySection() {
  const interfaceFont = useThemeStore((state) => state.interfaceFont)
  const interfaceSize = useThemeStore((state) => state.interfaceSize)
  const codeFont = useThemeStore((state) => state.codeFont)
  const codeSize = useThemeStore((state) => state.codeSize)
  const lighterText = useThemeStore((state) => state.lighterText)
  const update = useThemeStore((state) => state.update)

  return (
    <SettingsSection title={m.typography()}>
      <SettingsRow
        title={m.interface_font()}
        description={m.interface_font_description()}
        htmlFor='settings-interface-font'
        control={
          <OptionSelect
            id='settings-interface-font'
            options={INTERFACE_FONT_OPTIONS}
            value={interfaceFont}
            onChange={(value) => update({ interfaceFont: value })}
          />
        }
      />
      <SettingsRow
        title={m.interface_size()}
        description={m.interface_size_description()}
        control={
          <SizeSlider
            label={m.interface_size()}
            range={INTERFACE_SIZE}
            value={interfaceSize}
            onChange={(value) => update({ interfaceSize: value })}
          />
        }
      />
      <SettingsRow
        title={m.code_font()}
        description={m.code_font_description()}
        htmlFor='settings-code-font'
        control={
          <OptionSelect
            id='settings-code-font'
            options={CODE_FONT_OPTIONS}
            value={codeFont}
            onChange={(value) => update({ codeFont: value })}
          />
        }
      />
      <SettingsRow
        title={m.code_size()}
        description={m.code_size_description()}
        control={
          <SizeSlider
            label={m.code_size()}
            range={CODE_SIZE}
            value={codeSize}
            onChange={(value) => update({ codeSize: value })}
          />
        }
      />
      {IS_LINUX ? (
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
    </SettingsSection>
  )
}

function OptionSelect<T extends Appearance['interfaceFont' | 'codeFont']>({
  id,
  options,
  value,
  onChange
}: {
  id: string
  options: readonly { value: T; label: () => string }[]
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

function SizeSlider({
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
  return (
    <div className='flex w-56 items-center gap-3'>
      <Slider
        aria-label={label}
        min={range.min}
        max={range.max}
        step={1}
        value={[value]}
        onValueChange={(next) => {
          const size = Array.isArray(next) ? next[0] : next
          if (typeof size === 'number') onChange(size)
        }}
      />
      <span className='text-muted-foreground w-12 shrink-0 text-right text-xs tabular-nums'>
        {m.font_size_px({ size: value })}
      </span>
    </div>
  )
}
