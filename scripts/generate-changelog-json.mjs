// ============================================================================
//  generate-changelog-json.mjs - Extract recent releases from CHANGELOG.md
//  into a structured JSON for the in-app changelog viewer.
//
//  Usage:
//    node scripts/generate-changelog-json.mjs <changelog.md> <output.json> [maxVersions]
//
//  Output format:
//    [
//      { "version": "0.11.2", "date": "2026-10-08",
//        "entries": [ { "category": "时光机", "text": "..." }, ... ] },
//      ...
//    ]
// ============================================================================
import fs from 'node:fs'

const [changelogPath, outputPath, maxCount = '5'] = process.argv.slice(2)
if (!changelogPath || !outputPath) {
  throw new Error('Usage: node generate-changelog-json.mjs <changelog.md> <output.json> [maxVersions]')
}

const text = fs.readFileSync(changelogPath, 'utf8').replace(/\r\n/g, '\n')
const lines = text.split('\n')

const releases = []
let current = null

for (const line of lines) {
  const heading = /^##\s+\[(.*?)\]\s*-\s*(\d{4}-\d{2}-\d{2})/.exec(line)
  if (heading) {
    const [, version, date] = heading
    if (/unreleased/i.test(version)) { current = null; continue }
    current = { version, date, entries: [] }
    releases.push(current)
    continue
  }
  if (!current) continue
  const entry = /^\d+[.)]\s+\*\*(.+?)\*\*[:：]\s*(.+)$/.exec(line.trim())
  if (entry) {
    current.entries.push({ category: entry[1].trim(), text: entry[2].trim() })
  }
}

const result = releases.slice(0, parseInt(maxCount, 10))
fs.writeFileSync(outputPath, `${JSON.stringify(result, null, 2)}\n`)
console.log(`Generated ${outputPath} with ${result.length} releases`)
