import { cn } from '@hoshi/utils'
import type { CSSProperties, ReactNode } from 'react'
import {
  roleVariable,
  THEME_COLOR_ROLES,
  type ThemeAppearance,
  type ThemeColors
} from '@/state/themes'

function colorVariables(colors: ThemeColors): CSSProperties {
  const style: Record<string, string> = {}
  for (const role of THEME_COLOR_ROLES) style[roleVariable(role)] = colors[role]
  return style as CSSProperties
}

/** Paints its children with one theme's colors for one appearance. */
export function ThemeScope({
  colors,
  appearance,
  className,
  style,
  children
}: {
  colors: ThemeColors
  appearance: ThemeAppearance
  className?: string
  style?: CSSProperties
  children?: ReactNode
}) {
  return (
    <span
      aria-hidden
      className={cn(appearance, className)}
      style={{ ...colorVariables(colors), ...style }}
    >
      {children}
    </span>
  )
}

const ORB_LIGHT =
  '[background-image:radial-gradient(circle_at_72%_22%_in_oklab,var(--primary)_0%,color-mix(in_oklab,var(--primary)_72%,transparent)_28%,transparent_58%),radial-gradient(circle_at_18%_82%_in_oklab,color-mix(in_oklab,var(--ring)_45%,transparent)_0%,transparent_55%)]'
const ORB_DARK =
  '[background-image:radial-gradient(circle_at_28%_78%_in_oklab,var(--primary)_0%,color-mix(in_oklab,var(--primary)_62%,transparent)_28%,transparent_58%),radial-gradient(circle_at_82%_18%_in_oklab,color-mix(in_oklab,var(--ring)_45%,transparent)_0%,transparent_55%)]'

/** A round swatch of a theme: its background lit by its accent. */
export function ThemeOrb({
  colors,
  appearance,
  className
}: {
  colors: ThemeColors
  appearance: ThemeAppearance
  className?: string
}) {
  return (
    <ThemeScope
      colors={colors}
      appearance={appearance}
      className={cn(
        'border-background ring-foreground/10 relative block size-14 shrink-0 overflow-hidden rounded-full border-2 ring-1',
        className
      )}
    >
      <span
        className={cn(
          'bg-background absolute inset-0 scale-110 rounded-full blur-[3px]',
          appearance === 'dark' ? ORB_DARK : ORB_LIGHT
        )}
      />
    </ThemeScope>
  )
}

/** A small window drawn in a theme's colors; `clip` shows only one half of it. */
export function ThemeWireframe({
  colors,
  appearance,
  clip
}: {
  colors: ThemeColors
  appearance: ThemeAppearance
  clip?: 'left' | 'right'
}) {
  return (
    <ThemeScope
      colors={colors}
      appearance={appearance}
      className={cn(
        'bg-background absolute inset-0 flex overflow-hidden',
        clip === 'left' && '[clip-path:inset(0_50%_0_0)]',
        clip === 'right' && '[clip-path:inset(0_0_0_50%)]'
      )}
    >
      <span className='bg-sidebar border-sidebar-border flex w-1/4 flex-col gap-1 border-e p-1.5'>
        <span className='bg-sidebar-foreground/25 h-1 rounded-full' />
        <span className='bg-sidebar-accent h-1.5 rounded-sm' />
        <span className='bg-sidebar-foreground/15 h-1 w-2/3 rounded-full' />
      </span>
      <span className='flex flex-1 flex-col justify-end gap-1.5 p-2'>
        <span className='bg-secondary ms-auto h-2 w-1/2 rounded-md' />
        <span className='bg-foreground/15 h-1.5 w-3/4 rounded-full' />
        <span className='bg-primary h-1.5 w-1/2 rounded-full' />
      </span>
    </ThemeScope>
  )
}
