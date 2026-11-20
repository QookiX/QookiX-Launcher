// ============================================================================
//  fix-release-body.mjs - Repair the download tables inside a GitHub Release
//  body so every link points at an asset that actually exists.
//
//  Why: GitHub renames uploaded assets by replacing whitespace with "." —
//  local `QookiX Launcher_0.19.0_x64-setup.exe` becomes
//  `QookiX.Launcher_0.19.0_x64-setup.exe`. Notes generated from the local
//  filename (or with encodeURIComponent → `%20`) therefore link to a 404.
//  This script reads the release's REAL assets and rewrites the table rows
//  (`#### 类型 | 文件 | 下载`) in place. Anything outside those rows — the
//  changelog section, the tip — is left byte-for-byte untouched.
//
//  Usage:
//    GH_TOKEN=<token> GITHUB_REPOSITORY=owner/repo node scripts/fix-release-body.mjs <tag> [--dry-run]
//    GH_TOKEN=<token> GITHUB_REPOSITORY=owner/repo node scripts/fix-release-body.mjs --all [--dry-run]
//
//  Environment:
//    GH_TOKEN / GITHUB_TOKEN   token with write access to the repo's releases
//    GITHUB_REPOSITORY         e.g. "QookiX/QookiX-Launcher"
//
//  Exit code stays 0 when nothing needed fixing; `--dry-run` never writes.
// ============================================================================

import { githubReleases, releaseAssetUrl } from './shared/github-releases.mjs'

const token = process.env.GH_TOKEN || process.env.GITHUB_TOKEN
const repo = process.env.GITHUB_REPOSITORY
const args = process.argv.slice(2)
const dryRun = args.includes('--dry-run')
const all = args.includes('--all')
const strict = args.includes('--require-links')
const tag = args.find((a) => !a.startsWith('--'))

if (!token || !repo) throw new Error('GH_TOKEN and GITHUB_REPOSITORY are required')
if (!tag && !all) {
  throw new Error('Usage: fix-release-body.mjs <tag>|--all [--dry-run]')
}

const api = 'https://api.github.com'
const headers = {
  Authorization: `Bearer ${token}`,
  Accept: 'application/vnd.github+json',
  'X-GitHub-Api-Version': '2022-11-28',
}

/** 只匹配下载表格的数据行：`| 类型 | `文件` | [下载](url) |` */
const ROW = /^\|\s*([^|]*?)\s*\|\s*`([^`]+)`\s*\|\s*\[下载\]\(([^)]*)\)\s*\|\s*$/

/**
 * 本地文件名 → GitHub 实际使用的附件名。
 * GitHub 的规则是「空白字符换成 .」，例如 `QookiX Launcher_x.exe` → `QookiX.Launcher_x.exe`。
 */
const ghName = (name) => name.replace(/\s+/g, '.')

/** 用真实 asset 修一行；修不了就原样返回（返回 null 表示这一行不动） */
function fixRow(line, byName, releaseTag) {
  const m = ROW.exec(line)
  if (!m) return null
  const [, label, shown] = m
  // 正文上写的名字可能是本地名（带空格）也可能是已改好的名字
  const asset = byName.get(ghName(shown)) || byName.get(shown)
  if (!asset) return null
  const next = `| ${label} | \`${asset.name}\` | [下载](${releaseAssetUrl(asset, releaseTag)}) |`
  return next === line ? null : next
}

/** 返回 { body, changed, rows } */
function repairBody(body, assets, releaseTag) {
  if (!body) return { body, changed: 0, rows: 0, missing: [] }
  const byName = new Map(assets.map((a) => [a.name, a]))
  const missing = []
  let changed = 0
  let rows = 0
  const lines = body.replace(/\r\n/g, '\n').split('\n').map((line) => {
    const m = ROW.exec(line)
    if (!m) return line
    rows++
    if (!byName.has(ghName(m[2])) && !byName.has(m[2])) missing.push(m[2])
    const fixed = fixRow(line, byName, releaseTag)
    if (fixed) {
      changed++
      return fixed
    }
    return line
  })
  return { body: lines.join('\n'), changed, rows, missing }
}

async function get(path) {
  const res = await fetch(`${api}${path}`, { headers })
  if (!res.ok) throw new Error(`GET ${path} → ${res.status} ${await res.text()}`)
  return res.json()
}

async function patchBody(id, body) {
  const res = await fetch(`${api}/repos/${repo}/releases/${id}`, {
    method: 'PATCH',
    headers: { ...headers, 'Content-Type': 'application/json' },
    body: JSON.stringify({ body }),
  })
  if (!res.ok) throw new Error(`PATCH release ${id} → ${res.status} ${await res.text()}`)
  return res.json()
}

async function processRelease(release, releaseTag = release.tag_name) {
  const { body, changed, rows, missing } = repairBody(release.body, release.assets, releaseTag)
  if (strict && missing.length) throw new Error(`Release links do not have uploaded assets: ${missing.join(', ')}`)
  const label = `${releaseTag} (${rows} 行表格 / ${release.assets.length} 个附件)`
  if (!rows) {
    console.log(`  - ${label}: 没有下载表格，跳过`)
    return 0
  }
  if (!changed) {
    console.log(`  - ${label}: 链接都是对的`)
    return 0
  }
  console.log(`  - ${label}: 修好 ${changed} 行${dryRun ? '（dry-run，未写入）' : ''}`)
  if (!dryRun) await patchBody(release.id, body)
  return changed
}

let total = 0
if (all) {
  const releases = await get(`/repos/${repo}/releases?per_page=100&page=1`)
  const everything = [...releases]
  for (let page = 2; releases.length === 100; page++) {
    const next = await get(`/repos/${repo}/releases?per_page=100&page=${page}`)
    if (next.length === 0) break
    everything.push(...next)
    if (next.length < 100) break
  }
  console.log(`检查 ${everything.length} 个 Release${dryRun ? '（dry-run）' : ''}`)
  for (const rel of everything) total += await processRelease(rel)
} else {
  const client = githubReleases(repo, token)
  const id = process.env.QOOKIX_RELEASE_ID
  const rel = id === undefined ? await client.find(tag) : await client.getDraft(tag, Number(id))
  if (!rel) throw new Error(`Release ${tag} was not found`)
  total = await processRelease(rel, tag)
}

if (total === 0) console.log('无需修改，全部链接都指向真实附件。')
else if (!dryRun) console.log(`完成：共修正 ${total} 行下载链接。`)
