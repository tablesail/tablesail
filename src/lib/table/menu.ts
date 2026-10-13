export interface RowMenuItem {
  label?: string
  hint?: string
  danger?: boolean
  disabled?: boolean
  separator?: boolean
  onclick?: () => void
}
