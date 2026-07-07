import { useCallback, useEffect, useState } from 'react'
import { Loader2, Trash2, CheckCircle2, Plus, KeyRound } from 'lucide-react'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  addDebridService,
  listDebridServices,
  removeDebridService,
  setDefaultDebridService,
  type DebridKind,
  type DebridService,
} from '@/services/api'

interface DebridOption {
  kind: DebridKind
  label: string
  apiBase: string
  placeholder: string
  docs: string
}

const DEBRID_OPTIONS: DebridOption[] = [
  {
    kind: 'real_debrid',
    label: 'Real-Debrid',
    apiBase: 'https://api.real-debrid.com/rest/1.0',
    placeholder: 'Real-Debrid API token',
    docs: 'https://real-debrid.com/api',
  },
  {
    kind: 'all_debrid',
    label: 'AllDebrid',
    apiBase: 'https://api.alldebrid.com/v4',
    placeholder: 'AllDebrid API key',
    docs: 'https://alldebrid.com/apikeys',
  },
  {
    kind: 'premiumize',
    label: 'Premiumize',
    apiBase: 'https://www.premiumize.me/api',
    placeholder: 'Premiumize API key',
    docs: 'https://www.premiumize.me/account',
  },
  {
    kind: 'torbox',
    label: 'TorBox',
    apiBase: 'https://api.torbox.app',
    placeholder: 'TorBox API key',
    docs: 'https://torbox.app/settings',
  },
  {
    kind: 'offcloud',
    label: 'Offcloud',
    apiBase: 'https://offcloud.com/api',
    placeholder: 'Offcloud API key',
    docs: 'https://offcloud.com/#/account',
  },
  {
    kind: 'easydebrid',
    label: 'EasyDebrid',
    apiBase: 'https://easydebrid.ch/api/v1',
    placeholder: 'EasyDebrid API key',
    docs: 'https://easydebrid.ch',
  },
  {
    kind: 'linksnappy',
    label: 'LinkSnappy',
    apiBase: 'https://api.linksnappy.com/api',
    placeholder: 'LinkSnappy API key',
    docs: 'https://www.linksnappy.com/settings',
  },
  {
    kind: 'mega_debrid',
    label: 'Mega-Debrid',
    apiBase: 'https://api.mega-debrid.eu',
    placeholder: 'Mega-Debrid API key',
    docs: 'https://mega-debrid.eu',
  },
]

export function DebridServicesPanel() {
  const [services, setServices] = useState<DebridService[]>([])
  const [loading, setLoading] = useState(true)
  const [draftKind, setDraftKind] = useState<DebridKind>('real_debrid')
  const [draftKey, setDraftKey] = useState('')
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [removingKind, setRemovingKind] = useState<DebridKind | null>(null)

  const refresh = useCallback(async () => {
    setLoading(true)
    try {
      const list = await listDebridServices()
      setServices(list)
    } catch (e: any) {
      setError(String(e?.message || e || 'Could not load services.'))
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    void refresh()
  }, [refresh])

  const onAdd = useCallback(async () => {
    setError(null)
    if (!draftKey.trim()) {
      setError('Enter an API key.')
      return
    }
    setSubmitting(true)
    try {
      await addDebridService(draftKind, draftKey.trim())
      setDraftKey('')
      await refresh()
    } catch (e: any) {
      setError(translateError(String(e?.message || e || '')))
    } finally {
      setSubmitting(false)
    }
  }, [draftKind, draftKey, refresh])

  const onRemove = useCallback(
    async (kind: DebridKind) => {
      setRemovingKind(kind)
      try {
        await removeDebridService(kind)
        await refresh()
      } catch (e: any) {
        setError(String(e?.message || e || 'Could not remove service.'))
      } finally {
        setRemovingKind(null)
      }
    },
    [refresh]
  )

  const onSetDefault = useCallback(
    async (kind: DebridKind) => {
      try {
        await setDefaultDebridService(kind)
        await refresh()
      } catch (e: any) {
        setError(String(e?.message || e || 'Could not set default service.'))
      }
    },
    [refresh]
  )

  const draftOption =
    DEBRID_OPTIONS.find((o) => o.kind === draftKind) ?? DEBRID_OPTIONS[0]

  return (
    <section className="space-y-4">
      <div>
        <h3 className="text-lg font-semibold text-foreground">Debrid services</h3>
        <p className="text-sm text-muted-foreground">
          Configure one or more debrid services to resolve torrent streams from
          Stremio addons. Credentials stay on this device.
        </p>
      </div>

      {loading ? (
        <div className="flex items-center gap-2 text-sm text-muted-foreground p-3">
          <Loader2 className="size-4 animate-spin" /> Loading services…
        </div>
      ) : (
        <div className="space-y-2">
          {DEBRID_OPTIONS.map((opt) => {
            const svc = services.find((s) => s.kind === opt.kind)
            return (
              <div
                key={opt.kind}
                className={`p-3 rounded-xl border flex items-center gap-3 ${
                  svc ? 'bg-white/5 border-white/10' : 'bg-card border-border'
                }`}
              >
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <div
                      className={`size-2 rounded-full shrink-0 ${
                        svc ? 'bg-emerald-500' : 'bg-neutral-700'
                      }`}
                      title={svc ? 'Connected' : 'Not configured'}
                    />
                    <span className="text-sm font-medium truncate">{opt.label}</span>
                    {svc?.isDefault && (
                      <span className="text-[10px] px-1.5 py-0.5 rounded bg-white/10 text-white/60 uppercase tracking-wider">
                        Default
                      </span>
                    )}
                  </div>
                  <p className="text-xs text-muted-foreground truncate mt-0.5">
                    {svc ? `Signed in as ${svc.username}` : 'Not configured'}
                  </p>
                </div>
                <div className="flex items-center gap-1.5">
                  {svc && !svc.isDefault && (
                    <button
                      type="button"
                      onClick={() => void onSetDefault(opt.kind)}
                      className="text-xs px-2 py-1 rounded-lg bg-white/5 hover:bg-white/10 text-muted-foreground hover:text-foreground transition-colors"
                    >
                      Make default
                    </button>
                  )}
                  {svc && (
                    <button
                      type="button"
                      onClick={() => void onRemove(opt.kind)}
                      disabled={removingKind === opt.kind}
                      aria-label={`Remove ${opt.label}`}
                      className="p-1.5 rounded-lg hover:bg-destructive/10 text-muted-foreground hover:text-destructive transition-colors"
                    >
                      {removingKind === opt.kind ? (
                        <Loader2 className="size-3.5 animate-spin" />
                      ) : (
                        <Trash2 className="size-3.5" />
                      )}
                    </button>
                  )}
                </div>
              </div>
            )
          })}
        </div>
      )}

      {/* Add new */}
      <div className="p-4 rounded-xl bg-card border border-border space-y-3">
        <Label className="text-sm font-medium">Add Debrid Service</Label>

        <div className="flex flex-wrap gap-1.5">
          {DEBRID_OPTIONS.map((opt) => {
            const svc = services.find((s) => s.kind === opt.kind)
            const active = draftKind === opt.kind
            const isConfigured = Boolean(svc)
            return (
              <button
                key={opt.kind}
                type="button"
                aria-pressed={active}
                disabled={isConfigured}
                onClick={() => setDraftKind(opt.kind)}
                className={`inline-flex items-center gap-1.5 h-8 px-3 rounded-full text-xs font-medium border transition-colors ${
                  active
                    ? 'bg-white/10 border-white/20 text-foreground'
                    : 'bg-card border-border text-muted-foreground hover:text-foreground hover:bg-white/5'
                } ${isConfigured ? 'opacity-50 cursor-not-allowed' : ''}`}
              >
                {isConfigured ? (
                  <CheckCircle2 className="size-3 text-emerald-500" aria-hidden="true" />
                ) : null}
                <span>{opt.label}</span>
              </button>
            )
          })}
        </div>

        <div className="flex gap-2">
          <div className="relative flex-1">
            <KeyRound
              className="absolute left-3 top-1/2 -translate-y-1/2 size-3.5 text-muted-foreground pointer-events-none"
              aria-hidden="true"
            />
            <Input
              type="password"
              value={draftKey}
              onChange={(e) => setDraftKey(e.target.value)}
              placeholder={draftOption.placeholder}
              onKeyDown={(e) => {
                if (e.key === 'Enter') void onAdd()
              }}
              className="h-9 pl-9"
              aria-label={`${draftOption.label} API key`}
            />
          </div>
          <button
            type="button"
            onClick={() => void onAdd()}
            disabled={submitting || services.some((s) => s.kind === draftKind)}
            className="h-9 px-4 inline-flex items-center gap-1.5 rounded-xl bg-white/5 hover:bg-white/10 disabled:opacity-40 disabled:cursor-not-allowed text-sm font-medium transition-colors"
          >
            {submitting ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : (
              <Plus className="size-3.5" />
            )}
            <span>Add</span>
          </button>
        </div>

        <p className="text-[11px] text-muted-foreground">
          Need a key?{' '}
          <a
            href={draftOption.docs}
            target="_blank"
            rel="noreferrer"
            className="text-foreground/70 underline underline-offset-2 hover:text-foreground"
          >
            {draftOption.label} account settings
          </a>
        </p>

        {error && (
          <p
            role="alert"
            className="text-xs text-red-400 bg-red-950/30 border border-red-900/40 rounded-lg px-3 py-2"
          >
            {error}
          </p>
        )}
      </div>
    </section>
  )
}

function translateError(msg: string): string {
  const m = msg.toLowerCase()
  if (m.includes('status 401') || m.includes('status 403') || m.includes('invalid key')) {
    return "Couldn't validate that API key. Check it and try again."
  }
  return msg
}

export default DebridServicesPanel
