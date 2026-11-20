const VERSION = /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/
const DIGITS = /^[0-9]+$/
const MAX_CORE = 18446744073709551615n // same range as Rust semver::Version

function parseVersion(input) {
  if (typeof input !== 'string') return null
  const normalized = input.trim().replace(/^[vV]/, '')
  const match = VERSION.exec(normalized)
  if (!match) return null
  const core = match.slice(1, 4).map(BigInt)
  if (core.some((number) => number > MAX_CORE)) return null
  const pre = match[4]?.split('.') ?? []
  if (pre.some((id) => DIGITS.test(id) && id.length > 1 && id.startsWith('0'))) return null
  return { normalized, core, pre }
}

export function normalizeReleaseVersion(input) {
  return parseVersion(input)?.normalized ?? null
}

function compare(a, b) {
  return a === b ? 0 : a < b ? -1 : 1
}

export function compareReleaseVersions(left, right) {
  const a = parseVersion(left)
  const b = parseVersion(right)
  if (!a || !b) return null
  for (let i = 0; i < a.core.length; i++) {
    const result = compare(a.core[i], b.core[i])
    if (result) return result
  }
  if (!a.pre.length || !b.pre.length) return compare(Number(!a.pre.length), Number(!b.pre.length))
  for (let i = 0; i < Math.min(a.pre.length, b.pre.length); i++) {
    const x = a.pre[i]
    const y = b.pre[i]
    const xIsNumeric = DIGITS.test(x)
    const yIsNumeric = DIGITS.test(y)
    if (xIsNumeric !== yIsNumeric) return xIsNumeric ? -1 : 1
    const result = xIsNumeric ? compare(x.length, y.length) || compare(x, y) : compare(x, y)
    if (result) return result
  }
  return compare(a.pre.length, b.pre.length)
}
