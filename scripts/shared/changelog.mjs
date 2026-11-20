import { normalizeReleaseVersion } from './release-version.mjs'

export function parseChangelog(markdown, limit = 5) {
  const releases = []
  let current
  for (const line of markdown.split(/\r?\n/)) {
    if (line.startsWith('## ')) {
      current = undefined
      const heading = /^##\s+\[(.*?)\]\s*-\s*(\d{4}-\d{2}-\d{2})/.exec(line)
      if (!heading) continue
      const version = normalizeReleaseVersion(heading[1])
      if (!version) continue
      if (releases.length >= limit) break
      current = { version, date: heading[2], entries: [] }
      releases.push(current)
    } else if (current) {
      const match = /^\d+[.)]\s+\*\*(.+?)\*\*[:：]\s*(.+)$/.exec(line.trim())
      if (match) current.entries.push({ category: match[1].trim(), text: match[2].trim() })
    }
  }
  return releases
}
