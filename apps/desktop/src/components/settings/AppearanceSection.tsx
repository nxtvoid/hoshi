import { m } from '@/paraglide/messages'
import { Slider } from '@hoshi/ui/components/slider'
import { Switch } from '@hoshi/ui/components/switch'
import { ColorSchemeTiles, CreateThemeButton, ThemeGrid } from './ThemeLibrary'
import { SettingsPage, SettingsRow, SettingsSection } from './SettingsLayout'
import {
  type Appearance,
  CODE_SIZE,
  INTERFACE_SIZE,
  type SizeRange
} from '@/state/appearance'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue
} from '@hoshi/ui/components/select'
import { useThemeStore } from '@/state/theme-store'

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
          </div>
        </div>
        <ThemeGrid />
      </section>
      <TypographySection />
    </SettingsPage>
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
