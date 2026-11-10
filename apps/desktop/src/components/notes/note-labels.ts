import { m } from '@/paraglide/messages'

/** How many credentials a note leaves out when an agent reads it. */
export function hiddenCredentialsLabel(count: number) {
  return m.note_credentials_hidden({ count })
}
