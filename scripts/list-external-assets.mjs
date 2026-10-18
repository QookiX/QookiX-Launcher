// ============================================================================
//  list-external-assets.mjs - Print the release files that must NOT be uploaded
//  to the object storage bucket because they are served from GitHub Releases.
//
//  The bucket endpoint throttles large uploads (SlowDownWrite), so the Linux
//  AppImages are excluded from the mirror. `push-bucket.sh` consumes this list
//  to skip them; which suffixes count as external lives in
//  `shared/updater-targets.mjs` (`githubHosted`), the same source of truth the
//  manifest generators use.
//
//  Usage:
//    node scripts/list-external-assets.mjs <artifacts-dir>
// ============================================================================
import fs from 'node:fs'
import path from 'node:path'
import { isGithubHostedAsset } from './shared/updater-targets.mjs'

const dir = process.argv[2]
if (!dir) {
  throw new Error('Usage: node scripts/list-external-assets.mjs <artifacts-dir>')
}

const excluded = new Set()
for (const name of fs.readdirSync(dir)) {
  if (!isGithubHostedAsset(name)) continue
  excluded.add(name)
  excluded.add(`${name}.sig`)
}

for (const name of [...excluded].sort()) {
  console.log(name)
}
