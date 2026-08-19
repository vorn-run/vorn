/**
 * Turn terminal output into the plain text a reader can act on.
 *
 * What comes out of a PTY is not text with some colour in it, it is a stream of
 * drawing instructions. Storing it as-is gives a line buffer full of control
 * sequences, and every consumer of that buffer — the status parser, agent
 * history, and any client asking for a session's tail — reads noise.
 */

/**
 * A CSI sequence is `ESC [`, then parameter bytes, then intermediate bytes, then
 * one final byte. The parameter range is `0x30–0x3F`, which is the digits and
 * `;` *and* `? < = >`.
 *
 * The previous pattern accepted only `[0-9;]`, so every private-mode sequence
 * survived: `ESC[?25l` and `ESC[?25h`, which a TUI emits around each repaint to
 * hide and show the cursor, appeared in stored output as a literal `[?25l`. The
 * `ESC` is invisible when rendered, so the damage showed up as bracket noise
 * scattered through otherwise readable text, which reads as a rendering bug
 * rather than a stripping one.
 */
// eslint-disable-next-line no-control-regex
const CSI = /\x1b\[[\x30-\x3f]*[\x20-\x2f]*[\x40-\x7e]/g

/** OSC: `ESC ]` … terminated by BEL or ST. Window titles, hyperlinks. */
// eslint-disable-next-line no-control-regex
const OSC = /\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)/g

/** DCS, SOS, PM, APC: `ESC P/X/^/_` … ST. Rare, but unbounded if left in. */
// eslint-disable-next-line no-control-regex
const STRING_ESCAPES = /\x1b[P\x58\x5e\x5f][\s\S]*?(?:\x1b\\|\x07)/g

/** Charset selection and the two-character escapes that carry no payload. */
// eslint-disable-next-line no-control-regex
const SHORT_ESCAPES = /\x1b[()][\x20-\x2f]*[\x30-\x7e]|\x1b[=><78Mc]/g

const ESCAPES = new RegExp(
  [CSI.source, OSC.source, STRING_ESCAPES.source, SHORT_ESCAPES.source].join('|'),
  'g'
)

/**
 * Apply a carriage return the way a terminal does: by moving to the start of the
 * line, so what follows overwrites what came before.
 *
 * Deleting the `\r` instead — which is what this used to do — concatenates the
 * two, so an agent redrawing a spinner in place turned one line reading
 * `Working` into `WoWorkWorking`, and a progress bar became every frame it had
 * ever drawn, joined end to end.
 *
 * Only the text after the last `\r` survives, which is what the reader would
 * have seen on a real terminal.
 */
function applyCarriageReturns(line: string): string {
  const last = line.lastIndexOf('\r')
  return last === -1 ? line : line.slice(last + 1)
}

export function stripAnsi(data: string): string {
  return (
    data
      .replace(ESCAPES, '')
      // Before the rule below, or a CRLF line ending would be read as an
      // overwrite and delete the line it terminates.
      .replace(/\r\n/g, '\n')
      .split('\n')
      .map(applyCarriageReturns)
      .join('\n')
  )
}
