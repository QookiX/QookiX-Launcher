import fs from 'node:fs'
import path from 'node:path'

function walk(dir, out = []) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) walk(p, out)
    else if (/\.(vue|css)$/.test(e.name)) out.push(p)
  }
  return out
}

const counts = new Map()
for (const f of walk('src')) {
  const t = fs.readFileSync(f, 'utf8')
  for (const m of t.matchAll(/font-weight:\s*([0-9]+)/g)) {
    counts.set(m[1], (counts.get(m[1]) || 0) + 1)
  }
}
console.log([...counts.entries()].sort((a, b) => a[0] - b[0]).map(([v, c]) => `${v}×${c}`).join('  '))
