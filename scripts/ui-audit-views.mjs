import fs from 'node:fs'

const views = fs.readdirSync('src/views').filter((f) => f.endsWith('.vue'))
const rows = []
for (const f of views) {
  const t = fs.readFileSync('src/views/' + f, 'utf8')
  const usedTok = {
    fs: /var\(--fs-/.test(t),
    sp: /var\(--sp-/.test(t),
    grid: /qx-grid|--gap-grid/.test(t),
    card: /qx-card/.test(t),
    btn: /qx-btn/.test(t),
  }
  const mw = [...new Set([...t.matchAll(/max-width:\s*([0-9]+px)/g)].map((m) => m[1]))]
  const head = [...t.matchAll(/\.([a-z-]*(?:title|head)[a-z-]*)\s*\{[^}]*font-size:\s*([0-9.]+px)/g)]
    .map((m) => m[1] + '=' + m[2])
  rows.push({
    f,
    maxw: mw.join('/') || '—',
    head: [...new Set(head)].slice(0, 3).join(' '),
    used: Object.entries(usedTok).filter(([, v]) => v).map(([k]) => k).join(',') || '无',
  })
}
console.log('page'.padEnd(30), 'max-width'.padEnd(22), '共享类')
for (const r of rows) console.log(r.f.padEnd(30), r.maxw.padEnd(22), r.used, r.head ? '| ' + r.head : '')
