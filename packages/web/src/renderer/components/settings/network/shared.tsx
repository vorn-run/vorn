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

export function ScannableQRCode({ url }: { url: string }) {
  const [dataUrl, setDataUrl] = useState<string | null>(null)

  useEffect(() => {
    let mounted = true
    QRCode.toDataURL(url, {
      width: 640,
      margin: 2,
      errorCorrectionLevel: 'M',
      color: { dark: '#000000ff', light: '#ffffffff' }
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
      alt="Pairing code"
      title="Scan with the Vorn app"
      className="w-[212px] h-[212px] shrink-0 rounded-lg bg-white p-2"
    />
  )
}
