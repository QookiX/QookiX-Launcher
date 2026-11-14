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

const files = walk('src')
const c = {
  fsLit: 0, fsTok: 0,
  radLit: 0, radTok: 0,
  durLit: 0, durTok: 0,
  shadowHard: 0,
  accentHard: 0,
  hexTotal: 0,
}
const fsVals = new Map()
const radVals = new Map()
const filesByAccent = new Map()

const reFsLit = /font-size:\s*([0-9.]+)px/g
const reFsTok = /font-size:\s*var\(--fs-/g
const reRadLit = /border-radius:\s*([0-9.]+)px/g
const reRadTok = /border-radius:\s*var\(--r-/g
const reDurLit = /transition:[^;{}]*?\b([0-9.]+)s\b/g
const reDurTok = /var\(--dur-/g
const reBoxShadowHard = /box-shadow:\s*0[^;]*(rgba?\(0,\s*0,\s*0)/g
const reAccentHard = /#e89a4b|#e8a04b|rgba\(232,\s*154,\s*75/g
const reHex = /#[0-9a-fA-F]{3,8}\b/g

for (const f of files) {
  const t = fs.readFileSync(f, 'utf8')
  for (const m of t.matchAll(reFsLit)) {
    c.fsLit++
    fsVals.set(m[1], (fsVals.get(m[1]) || 0) + 1)
  }
  c.fsTok += [...t.matchAll(reFsTok)].length
  for (const m of t.matchAll(reRadLit)) {
    c.radLit++
    radVals.set(m[1], (radVals.get(m[1]) || 0) + 1)
  }
  c.radTok += [...t.matchAll(reRadTok)].length
  c.durLit += [...t.matchAll(reDurLit)].length
  c.durTok += [...t.matchAll(reDurTok)].length
  c.shadowHard += [...t.matchAll(reBoxShadowHard)].length
  const acc = [...t.matchAll(reAccentHard)].length
  if (acc) filesByAccent.set(f.replace(/\\/g, '/').replace('src/', ''), acc)
  c.accentHard += acc
  c.hexTotal += [...t.matchAll(reHex)].length
}

const pct = (a, b) => ((a / (a + b)) * 100).toFixed(1) + '%'
const top = (map, unit, n = 30) =>
  [...map.entries()].sort((x, y) => y[1] - x[1]).slice(0, n)
    .map(([v, k]) => `${v}${unit}×${k}`).join('  ')

console.log('=== 令牌接入率 ===')
console.log('字号  : 字面量', c.fsLit, '| 令牌', c.fsTok, '| 接入', pct(c.fsTok, c.fsLit))
console.log('圆角  : 字面量', c.radLit, '| 令牌', c.radTok, '| 接入', pct(c.radTok, c.radLit))
console.log('动效  : 字面量', c.durLit, '| 令牌', c.durTok, '| 接入', pct(c.durTok, c.durLit))
console.log()
console.log('字号取值分布 (' + fsVals.size + ' 种):')
console.log('  ' + top(fsVals, 'px'))
console.log()
console.log('圆角取值分布 (' + radVals.size + ' 种):')
console.log('  ' + top(radVals, 'px'))
console.log()
console.log('=== 颜色 ===')
console.log('硬编码琥珀色出现次数:', c.accentHard, '| 涉及文件', filesByAccent.size)
console.log('全仓 hex 字面量总数:', c.hexTotal)
console.log('深色硬投影 box-shadow:', c.shadowHard)
console.log()
if (filesByAccent.size) {
  console.log('硬编码琥珀色文件:')
  for (const [f, n] of [...filesByAccent.entries()].sort((a, b) => b[1] - a[1])) {
    console.log('  ' + String(n).padStart(3) + '  ' + f)
  }
}
