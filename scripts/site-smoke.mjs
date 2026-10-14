// Checks the live landing page after a deploy: the SEO basics, that the interactive preview
// runs, and a desktop and a phone screenshot (site-screenshots/) to look at.
//   node scripts/site-smoke.mjs https://tablesail.github.io/tablesail/
import { mkdirSync } from 'node:fs'
import { chromium } from '@playwright/test'

const base = (process.argv[2] ?? '').replace(/\/+$/, '')
if (!base) {
  console.error('usage: node scripts/site-smoke.mjs <site url>')
  process.exit(2)
}

const problems = []
const check = (ok, what) => {
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`)
  if (!ok) problems.push(what)
}

// GitHub's CDN can take a few seconds after a deploy to serve the new files.
async function get(path, tries = 12) {
  for (let i = 0; ; i++) {
    const res = await fetch(`${base}/${path}`)
    if (res.status === 200 || i >= tries) return res
    await new Promise((r) => setTimeout(r, 5000))
  }
}
const meta = (html, attr, name) => html.match(new RegExp(`<meta[^>]*${attr}="${name}"[^>]*content="([^"]*)"`, 'i'))?.[1] ?? ''

const home = await get('')
const html = await home.text()
check(home.status === 200, `GET / is 200 (got ${home.status})`)

const title = html.match(/<title>([^<]*)<\/title>/i)?.[1] ?? ''
const description = meta(html, 'name', 'description')
check(title.length > 0 && title.length <= 65, `title is 1-65 characters (${title.length}): ${title}`)
check(description.length >= 70 && description.length <= 160, `description is 70-160 characters (${description.length})`)
check(html.includes(`<link rel="canonical" href="${base}/">`), 'canonical points at the site root')
check((html.match(/<h1[\s>]/g) ?? []).length === 1, 'exactly one <h1>')
check(!/%%[A-Z_]+%%/.test(html), 'no placeholder left in the page')
check(meta(html, 'property', 'og:url') === `${base}/`, 'og:url is the site root')

const ogImage = meta(html, 'property', 'og:image')
check(ogImage.startsWith(`${base}/`), `og:image is absolute (${ogImage})`)
const og = await fetch(ogImage)
check(og.status === 200 && (og.headers.get('content-type') ?? '').startsWith('image/'), 'og:image is served as an image')

const ld = html.match(/<script type="application\/ld\+json">([\s\S]*?)<\/script>/)?.[1]
let graph = []
try {
  graph = JSON.parse(ld ?? '')['@graph'] ?? []
} catch {}
check(graph.some((n) => n['@type'] === 'SoftwareApplication'), 'JSON-LD has a SoftwareApplication')

const robots = await (await get('robots.txt')).text()
check(robots.includes(`Sitemap: ${base}/sitemap.xml`), 'robots.txt points at the sitemap')
const sitemap = await (await get('sitemap.xml')).text()
check(sitemap.includes(`<loc>${base}/</loc>`), 'sitemap.xml lists the home page')
for (const file of ['favicon.svg', 'favicon-32.png', 'apple-touch-icon.png', 'site.webmanifest']) {
  check((await fetch(`${base}/${file}`)).status === 200, `${file} is served`)
}
check((await fetch(`${base}/no-such-page`)).status === 404, 'unknown pages return 404')

// The page in a real browser.
mkdirSync('site-screenshots', { recursive: true })
const browser = await chromium.launch({ channel: 'chrome' })
for (const [name, viewport] of [['desktop', { width: 1440, height: 900 }], ['phone', { width: 390, height: 844 }]]) {
  const page = await browser.newPage({ viewport, colorScheme: 'dark' })
  const errors = []
  page.on('pageerror', (e) => errors.push(e.message))
  page.on('console', (m) => m.type() === 'error' && errors.push(m.text()))
  page.on('requestfailed', (r) => errors.push(`request failed: ${r.url()}`))
  await page.goto(`${base}/`, { waitUntil: 'networkidle' })
  check((await page.locator('#table-body tr').count()) > 0, `${name}: the interactive preview rendered its rows`)
  if (name === 'desktop') {
    await page.locator('[data-table="orders"]').click()
    check((await page.locator('#app-breadcrumb').textContent()) === 'orders', 'desktop: switching table in the preview works')
  }
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth)
  check(overflow <= 1, `${name}: no sideways scrolling (${overflow}px over)`)
  // Sections fade in as they are scrolled to. The page scrolls smoothly, so jump instantly or the
  // loop never gets down the page; then every section must have faded in.
  await page.evaluate(async () => {
    const step = Math.round(window.innerHeight * 0.6)
    for (let y = 0; y < document.documentElement.scrollHeight; y += step) {
      window.scrollTo({ top: y, behavior: 'instant' })
      await new Promise((r) => setTimeout(r, 150))
    }
    window.scrollTo({ top: 0, behavior: 'instant' })
  })
  await page.waitForTimeout(1000)
  const hidden = await page.locator('[data-reveal]:not(.visible)').count()
  check(hidden === 0, `${name}: every section fades in on scroll (${hidden} still hidden)`)
  await page.screenshot({ path: `site-screenshots/${name}.png`, fullPage: true })
  check(errors.length === 0, `${name}: no console errors or failed requests${errors.length ? ` (${errors[0]})` : ''}`)
  await page.close()
}
await browser.close()

if (problems.length) {
  console.error(`\n${problems.length} check(s) failed`)
  process.exit(1)
}
console.log('\nall checks passed')
