import { useState, useCallback, useId, type DragEvent, type ChangeEvent, type KeyboardEvent } from 'react'
import { open } from '@tauri-apps/api/dialog'
import { invoke } from '@tauri-apps/api/tauri'
import { Loader2, UploadCloud, Link2, ArrowRight, AlertTriangle, RotateCcw, X as XIcon, Film } from 'lucide-react'
import { StremioAddonSetupCard } from './StremioAddonSetupCard'
import type { StremioAddon } from '@/services/api'

interface AddonSetupWizardProps {
  onInstalled: (info: { url: string; version?: string | null }) => void
  onSaved: (url: string) => void
  onStremioInstalled?: (addon: StremioAddon) => void
  onError: (message: string) => void
  crashed: boolean
  onRetryRestart: () => Promise<void> | void
}

type UrlState = 'idle' | 'valid' | 'invalid'

function isLikelyUrl(value: string): boolean {
  if (!value) return false
  try {
    const u = new URL(value)
    return u.protocol === 'http:' || u.protocol === 'https:'
  } catch {
    return false
  }
}

// Inline film-reel mark. Hand-tuned so it reads as a film artifact, not a lucide fallback.
// Pure SVG; no font dependency; respects the monochrome palette.
function FilmReelMark({ size = 28 }: { size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 32 32"
      fill="none"
      aria-hidden="true"
      style={{ display: 'block' }}
    >
      <rect x="1.5" y="1.5" width="29" height="29" rx="7.5" stroke="currentColor" strokeWidth="1" opacity="0.35" />
      <circle cx="16" cy="16" r="9.25" stroke="currentColor" strokeWidth="1" />
      <circle cx="16" cy="16" r="1.6" fill="currentColor" />
      {/* Sprocket holes, evenly spaced at 60° intervals */}
      {[0, 60, 120, 180, 240, 300].map((deg) => {
        const r = 6
        const rad = (deg * Math.PI) / 180
        const x = 16 + r * Math.cos(rad)
        const y = 16 + r * Math.sin(rad)
        return <circle key={deg} cx={x} cy={y} r="1.4" fill="currentColor" opacity="0.85" />
      })}
    </svg>
  )
}

export function AddonSetupWizard({
  onInstalled,
  onSaved,
  onStremioInstalled,
  onError,
  crashed,
  onRetryRestart,
}: AddonSetupWizardProps) {
  const [binaryInstalling, setBinaryInstalling] = useState(false)
  const [setupAddonUrl, setSetupAddonUrl] = useState('')
  const [dragOver, setDragOver] = useState(false)
  const [savingUrl, setSavingUrl] = useState(false)
  const [restarting, setRestarting] = useState(false)

  const inputId = useId()
  const dropId = useId()

  const urlState: UrlState = setupAddonUrl.trim() === '' ? 'idle' : isLikelyUrl(setupAddonUrl.trim()) ? 'valid' : 'invalid'

  const installFromPath = useCallback(
    async (filePath: string) => {
      if (!filePath) return
      if (!filePath.toLowerCase().endsWith('.exe')) {
        onError('On Windows, the file must have an .exe extension.')
        return
      }
      setBinaryInstalling(true)
      try {
        const result = await invoke<{ url: string }>('install_addon_binary', {
          filePath,
          name: 'Custom Addon Binary',
        })
        let version: string | null = null
        try {
          version = await invoke<string | null>('get_addon_version', { url: result.url })
        } catch {
          version = null
        }
        onInstalled({ url: result.url, version })
      } catch (e: any) {
        const raw = String(e?.message || e || '')
        let message = raw
        if (raw.includes('--version')) message = 'That file is not a SlasshyVault-compatible addon binary.'
        else if (raw.toLowerCase().includes('too large')) message = 'File is too large. Addon binaries should be under 50 MB.'
        else if (raw.toLowerCase().includes('.exe')) message = 'On Windows, the file must have an .exe extension.'
        else if (raw.toLowerCase().includes('failed to start'))
          message = 'The binary failed to start. Check that no other instance is running and the port is free.'
        onError(message)
      } finally {
        setBinaryInstalling(false)
      }
    },
    [onInstalled, onError]
  )

  const onBrowseClick = useCallback(async () => {
    try {
      const selected = await open({
        multiple: false,
        filters: [{ name: 'Executable', extensions: ['exe'] }],
      })
      if (selected && typeof selected === 'string') {
        await installFromPath(selected)
      }
    } catch (e: any) {
      onError(String(e?.message || e || 'Could not open file picker.'))
    }
  }, [installFromPath, onError])

  const onDropFiles = useCallback(
    (e: DragEvent<HTMLDivElement>) => {
      e.preventDefault()
      e.stopPropagation()
      setDragOver(false)
      const file = e.dataTransfer.files?.[0] as (File & { path?: string }) | undefined
      if (!file) return
      const path = file.path || file.name
      void installFromPath(path)
    },
    [installFromPath]
  )

  const onDropDragOver = useCallback((e: DragEvent<HTMLDivElement>) => {
    e.preventDefault()
    e.stopPropagation()
    if (e.dataTransfer.types?.includes('Files')) setDragOver(true)
  }, [])

  const onDropDragLeave = useCallback((e: DragEvent<HTMLDivElement>) => {
    e.preventDefault()
    e.stopPropagation()
    setDragOver(false)
  }, [])

  const onSaveUrl = useCallback(async () => {
    const url = setupAddonUrl.trim()
    if (!isLikelyUrl(url)) {
      onError('Enter a valid http(s) URL.')
      return
    }
    setSavingUrl(true)
    try {
      await invoke('add_addon_source', { name: 'Default', url })
      onSaved(url)
    } catch (e: any) {
      onError(e?.message || 'Could not save the addon URL.')
    } finally {
      setSavingUrl(false)
    }
  }, [setupAddonUrl, onSaved, onError])

  const onUrlKey = useCallback(
    (e: KeyboardEvent<HTMLInputElement>) => {
      if (e.key === 'Enter') {
        e.preventDefault()
        void onSaveUrl()
      }
    },
    [onSaveUrl]
  )

  const onUrlChange = useCallback((e: ChangeEvent<HTMLInputElement>) => {
    setSetupAddonUrl(e.target.value)
  }, [])

  const onClearUrl = useCallback(() => setSetupAddonUrl(''), [])

  const onRetryClick = useCallback(async () => {
    setRestarting(true)
    try {
      await onRetryRestart()
    } finally {
      setRestarting(false)
    }
  }, [onRetryRestart])

  return (
    <div className="sv-setup-root h-full w-full overflow-y-auto">
      <div className="sv-setup-frame">
        {/* ── Identity rail ── */}
        <aside className="sv-setup-rail" aria-label="Setup overview">
          <div className="sv-rail-mark">
            <FilmReelMark size={28} />
          </div>

          <div className="sv-rail-eyebrow">
            <span className="sv-rail-eyebrow-line" />
            <span>Step 1 of 1</span>
            <span className="sv-rail-eyebrow-line" />
          </div>

          <h1 className="sv-rail-title">Set up streaming</h1>
          <p className="sv-rail-sub">
            SlasshyVault talks to your media sources through a small addon service. Point the app at one and the
            rest of the app comes online.
          </p>

          <ul className="sv-rail-list">
            <li>
              <span className="sv-rail-bullet">01</span>
              <div>
                <p className="sv-rail-list-title">Install a binary</p>
                <p className="sv-rail-list-sub">Drop a Windows addon binary you already have.</p>
              </div>
            </li>
            <li>
              <span className="sv-rail-bullet">02</span>
              <div>
                <p className="sv-rail-list-title">Paste a local server URL</p>
                <p className="sv-rail-list-sub">Or point at a SlasshyVault-compatible addon server.</p>
              </div>
            </li>
            <li>
              <span className="sv-rail-bullet">03</span>
              <div>
                <p className="sv-rail-list-title">Add a Stremio addon</p>
                <p className="sv-rail-list-sub">Paste any Stremio manifest link and we'll connect it.</p>
              </div>
            </li>
            <li>
              <span className="sv-rail-bullet">04</span>
              <div>
                <p className="sv-rail-list-title">Start watching</p>
                <p className="sv-rail-list-sub">Search, queue, and resume from one place.</p>
              </div>
            </li>
          </ul>

          <div className="sv-rail-foot">
            <Film aria-hidden="true" className="size-3 text-neutral-700" />
            <span>All sources are third-party. We don't host content.</span>
          </div>
        </aside>

        {/* ── Form column ── */}
        <section className="sv-setup-form" aria-label="Add your addon">
          {crashed && (
            <div className="sv-crash-row" role="alert">
              <AlertTriangle className="size-3.5 shrink-0" aria-hidden="true" />
              <span className="sv-crash-text">Addon binary crashed too many times.</span>
              <button
                type="button"
                onClick={onRetryClick}
                disabled={restarting}
                className="sv-crash-retry"
              >
                {restarting ? <Loader2 className="size-3 animate-spin" /> : <RotateCcw className="size-3" />}
                <span>{restarting ? 'Restarting' : 'Retry'}</span>
              </button>
            </div>
          )}

          {/* ── Install row ── */}
          <div className="sv-field" data-step="01">
            <div className="sv-field-head">
              <span className="sv-field-tag">01 · Install</span>
              <h2 className="sv-field-title">Drop a binary</h2>
              <p className="sv-field-sub">A signed SlasshyVault addon <code>.exe</code> will run locally on a private port.</p>
            </div>

            <div
              id={dropId}
              role="button"
              tabIndex={0}
              aria-label="Drop an addon binary here, or click to browse"
              aria-busy={binaryInstalling}
              onClick={() => !binaryInstalling && void onBrowseClick()}
              onKeyDown={(e) => {
                if (binaryInstalling) return
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault()
                  void onBrowseClick()
                }
              }}
              onDragOver={onDropDragOver}
              onDragEnter={onDropDragOver}
              onDragLeave={onDropDragLeave}
              onDrop={onDropFiles}
              data-state={binaryInstalling ? 'installing' : dragOver ? 'dragover' : 'idle'}
              className="sv-drop"
            >
              <div className="sv-drop-icon" aria-hidden="true">
                {binaryInstalling ? (
                  <Loader2 className="size-4 animate-spin" />
                ) : (
                  <UploadCloud className="size-4" />
                )}
              </div>
              <div className="sv-drop-copy">
                <p className="sv-drop-line1">
                  {binaryInstalling
                    ? 'Installing binary…'
                    : dragOver
                    ? 'Release to install'
                    : 'Drop the addon binary here'}
                </p>
                <p className="sv-drop-line2">
                  {binaryInstalling ? 'Hold on a moment' : 'or click to browse — .exe, no console window'}
                </p>
              </div>
              <div className="sv-drop-chev" aria-hidden="true">
                <ArrowRight className="size-3.5" />
              </div>
            </div>
          </div>

          {/* ── Divider with sprocket mark ── */}
          <div className="sv-divider" aria-hidden="true">
            <span className="sv-divider-line" />
            <span className="sv-divider-node">·</span>
            <span className="sv-divider-line" />
            <span className="sv-divider-label">or connect by URL</span>
            <span className="sv-divider-line" />
            <span className="sv-divider-node">·</span>
            <span className="sv-divider-line" />
          </div>

          {/* ── URL row ── */}
          <div className="sv-field" data-step="02">
            <div className="sv-field-head">
              <span className="sv-field-tag">02 · Connect</span>
              <h2 className="sv-field-title">Paste a local server URL</h2>
              <p className="sv-field-sub">Point at a SlasshyVault-compatible addon server already running on this machine or your network.</p>
            </div>

            <div className="sv-url" data-state={urlState}>
              <span className="sv-url-prefix" aria-hidden="true">
                <Link2 className="size-3.5" />
                <span>https://</span>
              </span>

              <input
                id={inputId}
                type="url"
                inputMode="url"
                spellCheck={false}
                autoComplete="off"
                autoCorrect="off"
                autoCapitalize="off"
                value={setupAddonUrl}
                onChange={onUrlChange}
                onKeyDown={onUrlKey}
                placeholder="your-addon-url.example.com"
                aria-label="Addon URL"
                aria-invalid={urlState === 'invalid'}
                className="sv-url-input"
              />

              {setupAddonUrl && (
                <button
                  type="button"
                  onClick={onClearUrl}
                  aria-label="Clear URL"
                  className="sv-url-clear"
                >
                  <XIcon className="size-3" />
                </button>
              )}

              <span className="sv-url-dot" aria-hidden="true" data-state={urlState} />
            </div>

            <button
              type="button"
              onClick={() => void onSaveUrl()}
              disabled={urlState !== 'valid' || savingUrl}
              className="sv-save"
            >
              {savingUrl ? (
                <Loader2 className="size-3.5 animate-spin" />
              ) : (
                <span className="sv-save-dot" aria-hidden="true" />
              )}
              <span>{savingUrl ? 'Connecting' : 'Save & Start Streaming'}</span>
              <ArrowRight className="sv-save-arrow size-3.5" />
            </button>
          </div>

          {/* ── Stremio row ── */}
          <StremioAddonSetupCard
            onInstalled={(addon) => {
              onStremioInstalled?.(addon)
            }}
            onError={onError}
          />
        </section>
      </div>
    </div>
  )
}

export default AddonSetupWizard
