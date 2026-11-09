import { m } from '@/paraglide/messages'
import type { ThemeError } from '@/state/themes'

export function themeErrorMessage(error: ThemeError): string {
  const subject = error.subject ?? ''
  switch (error.code) {
    case 'not_an_object':
      return m.theme_error_not_an_object()
    case 'unsupported_version':
      return m.theme_error_unsupported_version()
    case 'missing_name':
      return m.theme_error_missing_name()
    case 'missing_appearance':
      return m.theme_error_missing_appearance()
    case 'missing_colors':
      return m.theme_error_missing_colors()
    case 'invalid_id':
      return m.theme_error_invalid_id()
    case 'reserved_id':
      return m.theme_error_reserved_id({ id: subject })
    case 'unknown_role':
      return m.theme_error_unknown_role({ role: subject })
    case 'invalid_color':
      return m.theme_error_invalid_color({ role: subject })
    case 'no_colors':
      return m.theme_error_no_colors()
    case 'invalid_collection':
      return m.theme_error_invalid_collection()
    case 'invalid_variants':
      return m.theme_error_invalid_variants()
    case 'variant_repeats_appearance':
      return m.theme_error_variant_repeats_appearance({ appearance: subject })
    case 'already_installed':
      return m.theme_error_already_installed({ name: subject })
    case 'not_installed':
      return m.theme_error_not_installed({ name: subject })
    case 'empty_collection':
    case 'invalid_collection_themes':
      return m.theme_error_invalid_collection()
    case 'palette_taken':
      return error.appearance === 'dark'
        ? m.theme_error_dark_palette_taken({ name: subject })
        : m.theme_error_light_palette_taken({ name: subject })
    case 'palettes_taken':
      return m.theme_error_palettes_taken({ name: subject })
    case 'collection_changed':
      return m.theme_error_collection_changed()
    case 'vscode_no_background':
      return m.theme_error_vscode_no_background()
    case 'invalid_json':
      return m.theme_error_invalid_json()
    case 'no_compatible_themes':
      return m.open_vsx_no_themes()
  }
}
