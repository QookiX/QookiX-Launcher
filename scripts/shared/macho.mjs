import fs from 'node:fs'

const FAT_MAGIC = 0xcafebabe
const FAT_MAGIC_64 = 0xcafebabf
const MH_MAGIC_64 = 0xfeedfacf
const MH_EXECUTE = 2

function architecture(header, sliceLength) {
  const cpu = header.readUInt32LE(4)
  const subtype = header.readUInt32LE(8) & 0xffffff
  const commands = header.readUInt32LE(16)
  const commandBytes = header.readUInt32LE(20)
  if (header.readUInt32LE(0) !== MH_MAGIC_64 || header.readUInt32LE(12) !== MH_EXECUTE ||
      !commands || commands * 8 > commandBytes || 32 + commandBytes > sliceLength) {
    throw new Error('Invalid Mach-O executable header')
  }
  if (cpu === 0x01000007 && subtype === 3) return 'x86_64'
  if (cpu === 0x0100000c && (subtype === 0 || subtype === 1)) return 'arm64'
  throw new Error('Unsupported Mach-O executable architecture')
}

export function macExecutableArchitectures(filename) {
  const fd = fs.openSync(filename, 'r')
  try {
    const size = fs.fstatSync(fd).size
    const read = (length, offset) => {
      const buffer = Buffer.alloc(length)
      if (fs.readSync(fd, buffer, 0, length, offset) !== length) throw new Error('Truncated Mach-O executable')
      return buffer
    }
    const header = read(8, 0)
    const magic = header.readUInt32BE(0)
    if (magic !== FAT_MAGIC && magic !== FAT_MAGIC_64) return [architecture(read(32, 0), size)]

    const count = header.readUInt32BE(4)
    const wide = magic === FAT_MAGIC_64
    const stride = wide ? 32 : 20
    if (count < 1 || count > 16) throw new Error('Invalid Mach-O slice count')
    const table = read(count * stride, 8)
    const slices = []
    const arches = new Set()
    for (let start = 0; start < table.length; start += stride) {
      const offset = wide ? Number(table.readBigUInt64BE(start + 8)) : table.readUInt32BE(start + 8)
      const length = wide ? Number(table.readBigUInt64BE(start + 16)) : table.readUInt32BE(start + 12)
      if (!Number.isSafeInteger(offset) || !Number.isSafeInteger(length) || offset < 8 + table.length ||
          length < 32 || offset > size - length ||
          slices.some(([first, end]) => offset < end && first < offset + length)) {
        throw new Error('Invalid Mach-O slice bounds')
      }
      const slice = read(32, offset)
      if (slice.readUInt32LE(4) !== table.readUInt32BE(start) ||
          (slice.readUInt32LE(8) & 0xffffff) !== (table.readUInt32BE(start + 4) & 0xffffff)) {
        throw new Error('Mach-O slice architecture does not match its table entry')
      }
      const arch = architecture(slice, length)
      if (arches.has(arch)) throw new Error(`Duplicate Mach-O architecture: ${arch}`)
      arches.add(arch)
      slices.push([offset, offset + length])
    }
    return [...arches]
  } finally {
    fs.closeSync(fd)
  }
}
