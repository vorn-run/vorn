import { useState, useEffect, useCallback } from 'react'
import QRCode from 'qrcode'

/**
 * Pieces both halves of Remote Access use.
 *
 * Split out when the panel was divided into "share this machine" and "use another
 * machine": these are the parts that belong to neither side on its own.
 */

/**
 * Copy, with a fallback that matters here more than elsewhere.
 *
 * `navigator.clipboard` needs a secure context, and this panel's whole subject is a
 * plain-HTTP address on a LAN. When it is unavailable the prompt at least lets
 * someone select the text by hand.
 */
export function CopyButton({ text, label = 'Copy' }: { text: string; label?: string }) {
  const [copied, setCopied] = useState(false)

  const copy = useCallback(() => {
    // `navigator.clipboard` is absent entirely outside a secure context, so
    // reaching for `.writeText` throws before any promise exists and the rejection
    // handler below never runs. That is not a corner case here: this panel's whole
    // subject is a plain-HTTP address on a LAN, which is precisely where the API is
    // missing and the fallback is the only thing that works.
    const fallback = (): void => {
      window.prompt('Copy this URL:', text)
    }
    const done = (): void => {
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    }
    try {
      const clipboard = navigator.clipboard
      if (!clipboard?.writeText) return fallback()
      clipboard.writeText(text).then(done, fallback)
    } catch {
      fallback()
    }
  }, [text])

  return (
    <button
      onClick={copy}
      className="ml-2 shrink-0 px-2 py-1 text-xs rounded-md bg-white/[0.06] hover:bg-white/[0.1] text-gray-400 hover:text-white transition-colors"
      title="Copy to clipboard"
    >
      {copied ? 'Copied' : label}
    </button>
  )
}

/**
 * The address as something you can point a phone at.
 *
 * Sized to sit beside the address rather than below it. It used to be 200px in its
 * own centred block, which made scanning the panel mean scrolling past a picture to
 * reach the device list.
 */
export function QRCode128({ url }: { url: string }) {
  const [dataUrl, setDataUrl] = useState<string | null>(null)

  useEffect(() => {
    let mounted = true
    QRCode.toDataURL(url, {
      width: 256,
      margin: 1,
      color: { dark: '#ffffffFF', light: '#00000000' }
    })
      .then((data) => {
        if (mounted) setDataUrl(data)
      })
      .catch((err) => console.warn('[QRCode] generation failed:', err))
    return () => {
      mounted = false
    }
  }, [url])

  if (!dataUrl) return null

  return (
    <img
      src={dataUrl}
      alt="QR code to connect"
      title="Scan with your phone"
      className="w-[104px] h-[104px] shrink-0 rounded-md bg-white/[0.06] p-1.5 border border-white/[0.06]"
    />
  )
}
