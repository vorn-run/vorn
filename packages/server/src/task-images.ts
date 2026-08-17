import * as path from 'path'
import * as fs from 'fs'
import { randomUUID } from 'crypto'
import { getDataDir } from './database'

// Reads the resolved data directory rather than holding its own copy. Two
// independently-set holders meant every entry point had to remember to set
// both, and the CLI's token commands set only one.
function getImagesDir(): string {
  return path.join(getDataDir(), 'task-images')
}

function ensureDir(dir: string): void {
  if (!fs.existsSync(dir)) {
    fs.mkdirSync(dir, { recursive: true })
  }
}

/** Validate that an identifier contains only safe characters (alphanumeric, hyphens, underscores) */
function isSafeId(value: string): boolean {
  return /^[a-zA-Z0-9_-]+$/.test(value)
}

/** Validate that a filename contains only safe characters (alphanumeric, hyphens, underscores, dots) and no path separators */
function isSafeFilename(value: string): boolean {
  return /^[a-zA-Z0-9_.-]+$/.test(value) && !value.startsWith('.')
}

/** Resolve a path and verify it stays within the images directory */
function resolveSafePath(...segments: string[]): string {
  const imagesDir = getImagesDir()
  const resolved = path.resolve(imagesDir, ...segments)
  if (!resolved.startsWith(imagesDir + path.sep) && resolved !== imagesDir) {
    throw new Error('Path traversal detected')
  }
  return resolved
}

export function saveTaskImage(taskId: string, sourcePath: string): string {
  if (!isSafeId(taskId)) throw new Error(`Invalid taskId: ${taskId}`)

  const taskDir = resolveSafePath(taskId)
  ensureDir(taskDir)

  const ext = path.extname(sourcePath)
  const filename = `${randomUUID()}${ext}`
  const destPath = resolveSafePath(taskId, filename)

  fs.copyFileSync(sourcePath, destPath)
  return filename
}

export function deleteTaskImage(taskId: string, filename: string): void {
  if (!isSafeId(taskId)) throw new Error(`Invalid taskId: ${taskId}`)
  if (!isSafeFilename(filename)) throw new Error(`Invalid filename: ${filename}`)

  const filePath = resolveSafePath(taskId, filename)
  if (fs.existsSync(filePath)) {
    fs.unlinkSync(filePath)
  }
}

export function getTaskImagePath(taskId: string, filename: string): string {
  if (!isSafeId(taskId)) throw new Error(`Invalid taskId: ${taskId}`)
  if (!isSafeFilename(filename)) throw new Error(`Invalid filename: ${filename}`)

  return resolveSafePath(taskId, filename)
}

const MAX_IMAGE_SIZE = 10 * 1024 * 1024 // 10 MB decoded limit
const ALLOWED_IMAGE_EXTENSIONS = new Set(['.png', '.jpg', '.jpeg', '.gif', '.webp', '.bmp', '.svg'])

export function saveTaskImageFromBase64(
  taskId: string,
  base64Data: string,
  originalFilename: string
): string {
  if (!isSafeId(taskId)) throw new Error(`Invalid taskId: ${taskId}`)

  const ext = path.extname(originalFilename).toLowerCase() || '.png'
  if (!ALLOWED_IMAGE_EXTENSIONS.has(ext)) {
    throw new Error(`Unsupported image type: ${ext}`)
  }

  // Validate decoded size before allocating buffer
  const estimatedSize = Math.ceil(base64Data.length * 0.75)
  if (estimatedSize > MAX_IMAGE_SIZE) {
    throw new Error(`Image too large (${(estimatedSize / 1024 / 1024).toFixed(1)}MB). Max: 10MB`)
  }

  const taskDir = resolveSafePath(taskId)
  ensureDir(taskDir)

  const filename = `${randomUUID()}${ext}`
  const destPath = resolveSafePath(taskId, filename)

  const buffer = Buffer.from(base64Data, 'base64')
  fs.writeFileSync(destPath, buffer)
  return filename
}

export function cleanupTaskImages(taskId: string): void {
  if (!isSafeId(taskId)) throw new Error(`Invalid taskId: ${taskId}`)

  const taskDir = resolveSafePath(taskId)
  if (fs.existsSync(taskDir)) {
    fs.rmSync(taskDir, { recursive: true, force: true })
  }
}
