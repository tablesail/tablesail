import { mount } from 'svelte'
import './app.css'
import App from './App.svelte'

// WebKit (macOS/Linux web views) auto-capitalizes, autocorrects and spellchecks
// text fields by default, which mangles hostnames, usernames, SQL and cell values.
// Turn all of it off for every field, including ones created later.
document.addEventListener(
  'focusin',
  (e) => {
    const el = e.target
    if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || (el instanceof HTMLElement && el.isContentEditable)) {
      el.setAttribute('autocapitalize', 'off')
      el.setAttribute('autocorrect', 'off')
      el.setAttribute('autocomplete', 'off')
      el.spellcheck = false
    }
  },
  true,
)

// In a plain browser during development, stand in for the Tauri runtime with sample data.
if (import.meta.env.DEV && !('__TAURI_INTERNALS__' in window)) {
  await import('./dev/mockTauri')
}

const app = mount(App, {
  target: document.getElementById('app')!,
})

export default app
