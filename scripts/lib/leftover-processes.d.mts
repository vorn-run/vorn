export interface Process {
  pid: number
  argv0: string
  label: string
  text: string
}
export const NAMES: Set<string>
export const fold: (s: string) => string
export function spellings(roots: string[]): string[]
export function processes(): Process[]
export function leftoversMatching(named: (text: string) => boolean): Process[]
export function leftovers(roots: string[]): Process[]
export function leftAfter(find: () => Process[], seconds: number): Promise<Process[]>
export function endAll(left: Process[]): Promise<void>
