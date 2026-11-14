/**
 * Generate the application icon set without any image dependency.
 *
 * Writes RGBA PNGs by hand (IHDR/IDAT/IEND with zlib) and packs a multi-size
 * `icon.ico` where every entry is an embedded PNG — the format Windows has
 * accepted since Vista.
 *
 * Run: node scripts/gen-icons.mjs
 */
import { deflateSync } from 'node:zlib'
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const outDir = join(dirname(fileURLToPath(import.meta.url)), '..', 'apps', 'desktop', 'src-tauri', 'icons')
const PNG_SIZES = [16, 32, 48, 64, 128, 256, 512]
const ICO_SIZES = [16, 32, 48, 64, 128, 256]

// 主流程在文件末尾：crcTable 声明在下方，模块顶层先跑会撞 TDZ。

/** Paint one square icon: dark rounded plate, violet ring, bright core. */
function render(size) {
  const pixels = Buffer.alloc(size * size * 4)
  const center = (size - 1) / 2
  const radius = size / 2

  for (let y = 0; y < size; y += 1) {
    for (let x = 0; x < size; x += 1) {
      const offset = (y * size + x) * 4

      // Rounded-rectangle plate with a generous corner radius.
      const plate = roundedRectCoverage(x + 0.5 - center, y + 0.5 - center, radius, radius * 0.30)
      if (plate <= 0) continue

      const distance = Math.hypot(x + 0.5 - center, y + 0.5 - center) / radius

      // Core disc → ring → empty plate.
      const core = smoothstep(0.30, 0.24, distance)
      const ring = smoothstep(0.62, 0.56, distance) * smoothstep(0.46, 0.52, distance)

      const accent = Math.min(1, core * 1.0 + ring * 0.9)
      const r = mix(0.06, 0.39, accent) + core * 0.20
      const g = mix(0.07, 0.62, accent) + core * 0.10
      const b = mix(0.10, 0.98, accent) + core * 0.05

      pixels[offset] = clamp255(r * 255)
      pixels[offset + 1] = clamp255(g * 255)
      pixels[offset + 2] = clamp255(b * 255)
      pixels[offset + 3] = clamp255(plate * 255)
    }
  }
  return pixels
}

// Signed-distance coverage of a rounded square, sampled with 1px feathering.
function roundedRectCoverage(dx, dy, halfSize, cornerRadius) {
  const edge = halfSize - 2
  const qx = Math.abs(dx) - (edge - cornerRadius)
  const qy = Math.abs(dy) - (edge - cornerRadius)
  const outside = Math.hypot(Math.max(qx, 0), Math.max(qy, 0)) - cornerRadius
  const inside = Math.min(Math.max(qx, qy), 0)
  return smoothstep(1.0, -1.0, outside + inside)
}

function smoothstep(edge0, edge1, x) {
  const t = Math.min(1, Math.max(0, (x - edge0) / (edge1 - edge0)))
  return t * t * (3 - 2 * t)
}

function mix(a, b, t) {
  return a + (b - a) * t
}

function clamp255(value) {
  return Math.max(0, Math.min(255, Math.round(value)))
}

function encodePng(width, height, rgba) {
  const stride = width * 4
  const raw = Buffer.alloc((stride + 1) * height)
  for (let y = 0; y < height; y += 1) {
    raw[y * (stride + 1)] = 0 // filter: none
    rgba.copy(raw, y * (stride + 1) + 1, y * stride, (y + 1) * stride)
  }

  const ihdr = Buffer.alloc(13)
  ihdr.writeUInt32BE(width, 0)
  ihdr.writeUInt32BE(height, 4)
  ihdr[8] = 8 // bit depth
  ihdr[9] = 6 // truecolour with alpha
  ihdr[10] = 0
  ihdr[11] = 0
  ihdr[12] = 0

  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ])
}

function chunk(type, data) {
  const length = Buffer.alloc(4)
  length.writeUInt32BE(data.length, 0)
  const typed = Buffer.concat([Buffer.from(type, 'ascii'), data])
  const crc = Buffer.alloc(4)
  crc.writeUInt32BE(crc32(typed) >>> 0, 0)
  return Buffer.concat([length, typed, crc])
}

let crcTable = null
function crc32(buffer) {
  if (crcTable === null) {
    crcTable = new Uint32Array(256)
    for (let n = 0; n < 256; n += 1) {
      let c = n
      for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1
      crcTable[n] = c >>> 0
    }
  }
  let crc = 0xffffffff
  for (const byte of buffer) crc = crcTable[(crc ^ byte) & 0xff] ^ (crc >>> 8)
  return (crc ^ 0xffffffff) >>> 0
}

function encodeIco(entries) {
  const header = Buffer.alloc(6)
  header.writeUInt16LE(0, 0)
  header.writeUInt16LE(1, 2)
  header.writeUInt16LE(entries.length, 4)

  let offset = 6 + entries.length * 16
  const directory = Buffer.alloc(entries.length * 16)
  entries.forEach(({ size, png }, index) => {
    const at = index * 16
    directory[at] = size >= 256 ? 0 : size
    directory[at + 1] = size >= 256 ? 0 : size
    directory[at + 2] = 0
    directory[at + 3] = 0
    directory.writeUInt16LE(1, at + 4)
    directory.writeUInt16LE(32, at + 6)
    directory.writeUInt32LE(png.length, at + 8)
    directory.writeUInt32LE(offset, at + 12)
    offset += png.length
  })

  return Buffer.concat([header, directory, ...entries.map((entry) => entry.png)])
}

// ── entry point ────────────────────────────────────────────────────────────

mkdirSync(outDir, { recursive: true })

const pngs = new Map()
for (const size of PNG_SIZES) {
  const png = encodePng(size, size, render(size))
  pngs.set(size, png)
  const name = size === 512 ? 'icon.png' : `${size}x${size}.png`
  writeFileSync(join(outDir, name), png)
}
// macOS Retina slot uses the 256px asset.
writeFileSync(join(outDir, '128x128@2x.png'), pngs.get(256))

writeFileSync(join(outDir, 'icon.ico'), encodeIco(ICO_SIZES.map((size) => ({ size, png: pngs.get(size) }))))
console.log(`wrote ${PNG_SIZES.length} png + icon.ico to ${outDir}`)
