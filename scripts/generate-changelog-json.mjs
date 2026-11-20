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
import { parseChangelog } from './shared/changelog.mjs'

const [changelogPath, outputPath, maxCount = '5'] = process.argv.slice(2)
if (!changelogPath || !outputPath) {
  throw new Error('Usage: node generate-changelog-json.mjs <changelog.md> <output.json> [maxVersions]')
}

const count = Number(maxCount)
if (!Number.isSafeInteger(count) || count < 1) throw new Error('maxVersions must be a positive integer')
const result = parseChangelog(fs.readFileSync(changelogPath, 'utf8'), count)
fs.writeFileSync(outputPath, `${JSON.stringify(result, null, 2)}\n`)
console.log(`Generated ${outputPath} with ${result.length} releases`)
