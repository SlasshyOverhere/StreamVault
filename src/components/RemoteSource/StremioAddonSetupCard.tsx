import { useCallback, useState, type KeyboardEvent } from 'react'
import { Loader2, Plug, ArrowRight, AlertTriangle, X as XIcon, Check } from 'lucide-react'
import { addStremioAddon, type StremioAddon } from '@/services/api'

interface StremioAddonSetupCardProps {
  onInstalled: (addon: StremioAddon) => void
  onError: (message: string) => void
}

type UrlState = 'idle' | 'valid' | 'invalid'

function isLikelyUrl(value: string): boolean {
  if (!value) return false
  const normalized = value.trim().toLowerCase().startsWith('stremio://')
    ? value.trim().replace(/^stremio:\/\//i, 'https://')
    : value.trim()
  try {
    const u = new URL(normalized)
    return u.protocol === 'http:' || u.protocol === 'https:'
  } catch {
    return false
  }
}

export function StremioAddonSetupCard({ onInstalled, onError }: StremioAddonSetupCardProps) {
  const [url, setUrl] = useState('')
  const [installing, setInstalling] = useState(false)
  const [installedPreview, setInstalledPreview] = useState<StremioAddon | null>(null)
  const [inlineError, setInlineError] = useState<string | null>(null)

  const urlState: UrlState = url.trim() === '' ? 'idle' : isLikelyUrl(url) ? 'valid' : 'invalid'

  const onInstall = useCallback(async () => {
    const raw = url.trim()
    if (!isLikelyUrl(raw)) {
      onError('Enter a valid Stremio addon URL (http(s) or stremio://).')
      return
    }
    setInlineError(null)
    setInstalling(true)
    try {
      const addon = await addStremioAddon(raw)
      setInstalledPreview(addon)
      onInstalled(addon)
    } catch (e: any) {
      const msg = String(e?.message || e || 'Could not install addon.')
      const friendly = translateError(msg)
      setInlineError(friendly)
      onError(friendly)
    } finally {
      setInstalling(false)
    }
  }, [url, onInstalled, onError])

  const onKey = useCallback(
    (e: KeyboardEvent<HTMLInputElement>) => {
      if (e.key === 'Enter') {
        e.preventDefault()
        void onInstall()
      }
    },
    [onInstall]
  )

  const onClear = useCallback(() => {
    setUrl('')
    setInstalledPreview(null)
    setInlineError(null)
  }, [])

  return (
    <div className="sv-field" data-step="03">
      <div className="sv-field-head">
        <span className="sv-field-tag">03 · Stremio</span>
        <h2 className="sv-field-title">Add a Stremio addon</h2>
        <p className="sv-field-sub">
          Paste any Stremio addon link. We'll fetch its <code>manifest.json</code> and connect.
        </p>
      </div>

      <div className="sv-url" data-state={urlState}>
        <span className="sv-url-prefix" aria-hidden="true">
          <Plug className="size-3.5" />
          <span>https://</span>
        </span>

        <input
          type="url"
          inputMode="url"
          spellCheck={false}
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
          value={url}
          onChange={(e) => { setUrl(e.target.value); setInlineError(null) }}
          onKeyDown={onKey}
          placeholder="stremio-addon.example.com/manifest.json"
          aria-label="Stremio addon URL"
          aria-invalid={urlState === 'invalid'}
          className="sv-url-input"
        />

        {url && (
          <button type="button" onClick={onClear} aria-label="Clear URL" className="sv-url-clear">
            <XIcon className="size-3" />
          </button>
        )}

        <span className="sv-url-dot" aria-hidden="true" data-state={urlState} />
      </div>

      <button
        type="button"
        onClick={() => void onInstall()}
        disabled={urlState !== 'valid' || installing}
        className="sv-save"
      >
        {installing ? (
          <Loader2 className="size-3.5 animate-spin" />
        ) : (
          <span className="sv-save-dot" aria-hidden="true" />
        )}
        <span>{installing ? 'Installing' : 'Fetch & Install'}</span>
        <ArrowRight className="sv-save-arrow size-3.5" />
      </button>

      {installedPreview && (
        <div className="sv-stremio-preview" role="status">
          <Check className="size-3.5 shrink-0" aria-hidden="true" />
          <div>
            <p className="sv-stremio-preview-name">{installedPreview.name}</p>
            <p className="sv-stremio-preview-sub">
              v{installedPreview.version} · {installedPreview.types.join(', ') || 'no types'} ·{' '}
              {installedPreview.resources.length} resource
              {installedPreview.resources.length === 1 ? '' : 's'}
            </p>
          </div>
        </div>
      )}

      {urlState === 'invalid' && (
        <div className="sv-stremio-hint" role="note">
          <AlertTriangle className="size-3 shrink-0" aria-hidden="true" />
          <span>URL must start with http://, https://, or stremio://</span>
        </div>
      )}

      {inlineError && (
        <div className="text-xs text-red-400 bg-red-950/30 border border-red-900/40 rounded-lg px-3 py-2" role="alert">
          {inlineError}
        </div>
      )}
    </div>
  )
}

function translateError(msg: string): string {
  const m = msg.toLowerCase()
  if (m.includes('unreachable') || m.includes('status')) {
    return "Couldn't reach the addon's manifest. Check the URL or your network."
  }
  if (m.includes('invalid json') || m.includes('missing')) {
    return "This doesn't look like a Stremio addon. It must serve a valid manifest.json."
  }
  if (m.includes('no resources')) {
    return "This manifest doesn't declare any resources, so there's nothing to install."
  }
  return msg
}

export default StremioAddonSetupCard
