/**
 * A cron expression as a person would say it.
 *
 * Only the shapes the connector seeding produces are translated; anything else
 * is shown verbatim rather than guessed at, since a wrong reading of a schedule
 * is worse than an unfamiliar one.
 */
export function humanCron(cron: string): string {
  const m = cron.match(/^\*\/(\d+) \* \* \* \*$/)
  if (m) return `every ${m[1]} minute${m[1] === '1' ? '' : 's'}`
  if (cron === '* * * * *') return 'every minute'
  return cron
}
