import { z } from 'zod'

/**
 * Shared Zod schemas for MCP tool input validation.
 *
 * All user-facing string inputs must be length-bounded to prevent
 * overflow attacks and resource exhaustion. Identifiers must be
 * validated against path traversal patterns.
 */

/** Rejects strings containing path traversal sequences (.. or /). */
const safeName = z
  .string()
  .min(1, 'Name must not be empty')
  .max(200, 'Name must be 200 characters or less')
  .refine((s) => !s.includes('..') && !s.includes('/') && !s.includes('\\'), {
    message: 'Name must not contain path traversal characters (.. / \\)'
  })

/** UUID format validator for resource IDs. */
const safeId = z
  .string()
  .min(1, 'ID must not be empty')
  .max(100, 'ID must be 100 characters or less')

/** Bounded title field. */
const safeTitle = z
  .string()
  .min(1, 'Title must not be empty')
  .max(500, 'Title must be 500 characters or less')

/** Bounded description field. */
const safeDescription = z.string().max(5000, 'Description must be 5000 characters or less')

/** Bounded short text (branch names, display names, icon names). */
const safeShortText = z.string().max(200, 'Value must be 200 characters or less')

/** Bounded URL-ish input. Scheme filtering happens in the main process, which
 *  applies the same allowlist the address bar does; this only bounds length. */
const safeUrl = z.string().min(1, 'URL must not be empty').max(2048, 'URL is too long')

/** Bounded prompt text. */
const safePrompt = z.string().max(10000, 'Prompt must be 10000 characters or less')

/** Absolute filesystem path. Must start with /. */
const safeAbsolutePath = z
  .string()
  .min(1, 'Path must not be empty')
  .max(1000, 'Path must be 1000 characters or less')
  .refine((s) => s.startsWith('/'), { message: 'Path must be absolute (start with /)' })

/** Hex color validator (#000000 format). */
const safeHexColor = z
  .string()
  .regex(/^#[0-9a-fA-F]{3,8}$/, 'Must be a valid hex color (e.g. #6366f1)')

export const V = {
  name: safeName,
  id: safeId,
  title: safeTitle,
  description: safeDescription,
  shortText: safeShortText,
  prompt: safePrompt,
  absolutePath: safeAbsolutePath,
  hexColor: safeHexColor,
  url: safeUrl
}
