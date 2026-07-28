export interface ShellOptions {
  /** Replace the shell's own prompt with nothing, so blocks own the layout. */
  minimalPrompt: boolean
  /** The user's real ZDOTDIR, for the zsh shim to hand back. */
  userZdotdir: string
}

export interface ShellSetup {
  env: Record<string, string>
  /**
   * Launch arguments, replacing the defaults when non-null.
   *
   * bash and PowerShell have no environment variable that injects
   * initialisation — bash can only be told with `--rcfile`, and PowerShell only
   * by running something on its command line. zsh, fish and cmd need no change
   * here and leave this null.
   */
  args: string[] | null
}
