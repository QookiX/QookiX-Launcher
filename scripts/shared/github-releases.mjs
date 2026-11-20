import { createHash } from 'node:crypto'
import fs from 'node:fs'

export function githubReleases(repo, token, request = fetch) {
  if (!/^[-.\w]+\/[-.\w]+$/.test(repo ?? '') || !token) {
    throw new Error('GITHUB_REPOSITORY and GH_TOKEN are required')
  }
  const endpoint = `https://api.github.com/repos/${repo}/releases`
  const headers = { Authorization: `Bearer ${token}`, 'X-GitHub-Api-Version': '2022-11-28' }
  async function send(url, { accept = 'application/vnd.github+json', ...options } = {}) {
    return request(url, {
      ...options,
      headers: { ...headers, Accept: accept, ...options.headers },
      signal: AbortSignal.timeout(120_000),
      redirect: 'follow',
    })
  }

  async function json(url, options) {
    const response = await send(url, options)
    if (!response.ok) throw new Error(`GitHub release request failed: ${response.status}`)
    return response.status === 204 ? null : response.json()
  }

  async function find(tag) {
    const response = await send(`${endpoint}/tags/${encodeURIComponent(tag)}`)
    if (response.ok) return response.json()
    if (response.status !== 404) throw new Error(`Could not locate release: HTTP ${response.status}`)
    // Authorized list includes drafts, even when lookup by tag returns 404.
    for (let page = 1; page <= 100; page++) {
      const releases = await json(`${endpoint}?per_page=100&page=${page}`)
      const release = releases.find((entry) => entry.tag_name === tag)
      if (release || releases.length < 100) return release
    }
    throw new Error('Release history exceeds the lookup limit')
  }

  async function download(asset) {
    if (!Number.isSafeInteger(asset.id) || asset.id <= 0) throw new Error('Invalid GitHub asset ID')
    const response = await send(`${endpoint}/assets/${asset.id}`, { accept: 'application/octet-stream' })
    if (!response.ok || !response.body) throw new Error(`Could not download ${asset.name}: HTTP ${response.status}`)
    return response
  }

  const digest = async (asset) => {
    if (/^sha256:[a-f0-9]{64}$/.test(asset.digest ?? '')) return asset.digest
    return sha256((await download(asset)).body)
  }

  return {
    find,
    digest,
    async text(asset, limit = 256 * 1024) {
      if (asset.size > limit) throw new Error(`${asset.name} exceeds the metadata size limit`)
      const chunks = []
      let size = 0
      const response = await download(asset)
      for await (const chunk of response.body) {
        size += chunk.length
        if (size > limit) throw new Error(`${asset.name} exceeds the metadata size limit`)
        chunks.push(chunk)
      }
      return Buffer.concat(chunks).toString('utf8')
    },
    edit: (id, body) => json(`${endpoint}/${id}`, {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    }),
    remove: (id) => json(`${endpoint}/assets/${id}`, { method: 'DELETE' }),
    async upload(id, name, bytes) {
      const created = await json(`https://uploads.github.com/repos/${repo}/releases/${id}/assets?name=${encodeURIComponent(name)}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/octet-stream' },
        body: bytes,
      })
      const actual = await digest(created)
      if (created.state !== 'uploaded' || created.name !== name || created.size !== bytes.length
          || actual !== await sha256([bytes])) {
        throw new Error(`GitHub could not verify the uploaded ${name}`)
      }
      return created
    },
  }
}

export async function sha256(chunks) {
  const hash = createHash('sha256')
  for await (const chunk of chunks) hash.update(chunk)
  return `sha256:${hash.digest('hex')}`
}

export function fileDigest(filename) {
  return sha256(fs.createReadStream(filename))
}
