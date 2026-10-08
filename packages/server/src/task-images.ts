import * as path from 'path'
import { getDataDir } from './database'

// Reads the resolved data directory rather than holding its own copy.
function getImagesDir(): string {
  return path.join(getDataDir(), 'task-images')
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

/** Where the image route reads a task's image; vornd writes them. */
export function getTaskImagePath(taskId: string, filename: string): string {
  if (!isSafeId(taskId)) throw new Error(`Invalid taskId: ${taskId}`)
  if (!isSafeFilename(filename)) throw new Error(`Invalid filename: ${filename}`)

  return resolveSafePath(taskId, filename)
}
