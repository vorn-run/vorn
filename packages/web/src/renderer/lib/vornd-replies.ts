import type { IDisposable, Terminal } from '@xterm/xterm'

/**
 * Keeps xterm.js from answering a program's terminal queries for sessions
 * vornd holds.
 *
 * vornd parses every session it holds and answers its queries itself, live
 * and once. xterm.js answers the same queries from what it parsed, and its
 * answers go back to the program as typed input -- so with the desktop and a
 * phone attached, a device-attributes query used to get three answers and the
 * program read the extras as keystrokes. Each handler here claims the query
 * when `answeredElsewhere()` says vornd answers this terminal, and otherwise
 * lets xterm.js handle it as before, so a terminal the Node server holds is
 * unaffected.
 *
 * The queries are the ones xterm.js answers: device attributes (primary,
 * secondary, tertiary), status and cursor reports, mode reports, its version,
 * setting reports, and colour queries. A colour *set* is never claimed: the
 * client still draws with it.
 */
export function swallowQueries(term: Terminal, answeredElsewhere: () => boolean): IDisposable[] {
  const claim = (): boolean => answeredElsewhere()
  const parser = term.parser
  const csi = [
    { final: 'c' }, // DA1
    { prefix: '>', final: 'c' }, // DA2
    { prefix: '=', final: 'c' }, // DA3
    { final: 'n' }, // DSR 5, 6
    { prefix: '?', final: 'n' }, // DECDSR
    { intermediates: '$', final: 'p' }, // DECRQM, ANSI modes
    { prefix: '?', intermediates: '$', final: 'p' }, // DECRQM, DEC modes
    { prefix: '>', final: 'q' } // XTVERSION
  ]
  const handlers: IDisposable[] = csi.map((id) => parser.registerCsiHandler(id, claim))
  // DECRQSS: a setting report.
  handlers.push(parser.registerDcsHandler({ intermediates: '$', final: 'q' }, claim))
  // Colour queries carry `?` where a set carries a colour.
  const colourQuery = (data: string): boolean => claim() && data.split(';').includes('?')
  for (const ident of [4, 10, 11, 12]) {
    handlers.push(parser.registerOscHandler(ident, colourQuery))
  }
  return handlers
}
