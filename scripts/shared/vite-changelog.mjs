import { readFile } from 'node:fs/promises'
import { parseChangelog } from './changelog.mjs'

export function changelogPlugin() {
  return {
    name: 'qookix-changelog',
    async load(id) {
      const suffix = '.md?releases'
      if (!id.endsWith(suffix)) return null
      const filename = id.slice(0, -'?releases'.length)
      this.addWatchFile(filename)
      const releases = parseChangelog(await readFile(filename, 'utf8'))
      return `export default ${JSON.stringify(releases)}`
    },
  }
}
