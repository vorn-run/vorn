/**
 * Asked for when the server refuses the stored credential, or there isn't one.
 *
 * Deliberately plain DOM and no imports: it runs on the path where the app has
 * not loaded, so pulling in React and the stylesheet to show one input would mean
 * an unauthenticated visitor downloads the whole application first.
 *
 * A pasted token is the crude version on purpose. Pairing — a QR code from the
 * desktop, a device list to revoke from — is the follow-on work; this exists so
 * the web client keeps working the moment authentication becomes mandatory.
 */
const SURFACE = 'padding:10px 12px;border-radius:4px;background:#1c1c20;color:#faf9f7'
const MONO = 'font-family:ui-monospace,SFMono-Regular,Menlo,monospace'
const EDGE = 'border:1px solid rgba(255,255,255,0.14)'

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  style: string,
  text?: string
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag)
  node.setAttribute('style', style)
  if (text) node.textContent = text
  return node
}

export function renderTokenPrompt(onSubmit: (token: string) => void): void {
  const root = document.getElementById('root')
  if (!root) return

  root.textContent = ''
  root.setAttribute(
    'style',
    'min-height:100vh;display:flex;align-items:center;justify-content:center;' +
      'background:#0d0d0f;color:#faf9f7;font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif'
  )

  const card = document.createElement('form')
  card.setAttribute(
    'style',
    'display:flex;flex-direction:column;gap:16px;width:min(420px,calc(100% - 48px))'
  )

  const heading = el('h1', 'margin:0;font-size:20px;font-weight:600', 'Connect to Vorn')

  const blurb = el(
    'p',
    'margin:0;color:rgba(255,255,255,0.55);font-size:14px;line-height:1.6',
    'This server needs a device token. Create one on the machine running Vorn:'
  )

  const command = el(
    'code',
    `display:block;${SURFACE};${MONO};font-size:12.5px;overflow-x:auto`,
    'vorn-server token create --name "This device"'
  )

  const input = document.createElement('input')
  input.type = 'password'
  input.autocomplete = 'off'
  input.placeholder = 'vorn_…'
  input.setAttribute('aria-label', 'Device token')
  input.setAttribute(
    'style',
    `padding:10px 12px;border-radius:4px;${EDGE};background:#141416;color:#faf9f7;${MONO};font-size:13px`
  )

  const button = el('button', `${SURFACE};${EDGE};font-size:14px;cursor:pointer`, 'Connect')
  button.type = 'submit'

  card.append(heading, blurb, command, input, button)
  card.addEventListener('submit', (event) => {
    event.preventDefault()
    const token = input.value.trim()
    if (token.length > 0) onSubmit(token)
  })

  root.append(card)
  input.focus()
}
