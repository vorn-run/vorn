/**
 * Turn what the user typed into a command and arguments.
 *
 * A package name is the common case and runs through `npx -y`. A full command
 * is accepted too, so a connector can be run from a local checkout while it is
 * being developed.
 */
export function parseLaunchSpec(input: string): { command: string; args: string[] } {
  const trimmed = input.trim()
  const parts = trimmed.split(/\s+/)
  // A lone token is a package name unless it looks like a path or an
  // executable the user clearly meant to run directly.
  if (parts.length === 1 && !/^[./~]|^[a-zA-Z]:\\/.test(trimmed)) {
    return { command: 'npx', args: ['-y', trimmed] }
  }
  return { command: parts[0], args: parts.slice(1) }
}
