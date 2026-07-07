import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  fetchStremioCatalog,
  fetchStremioStreams,
  listStremioAddons,
  removeStremioAddon,
  resolveStremioStream,
  type StremioAddon,
  type StremioCatalogRef,
  type StremioStreamLike,
} from '@/services/api'
import {
  toRemoteStreamData,
  streamNeedsDebrid,
  type StremioRawStream,
} from './StremioStreamAdapter'
import { Loader2, Trash2, AlertCircle, RefreshCw, Plug, Search } from 'lucide-react'
import type { RemoteStreamData } from './remote.types'

interface StremioAddonsViewProps {
  onPlayStream: (stream: RemoteStreamData) => void
}

export function StremioAddonsView({ onPlayStream }: StremioAddonsViewProps) {
  const [addons, setAddons] = useState<StremioAddon[]>([])
  const [loading, setLoading] = useState(true)
  const [activeId, setActiveId] = useState<string | null>(null)
  const [activeCatalog, setActiveCatalog] = useState<StremioCatalogRef | null>(null)
  const [metas, setMetas] = useState<unknown[]>([])
  const [catalogLoading, setCatalogLoading] = useState(false)
  const [activeMeta, setActiveMeta] = useState<any | null>(null)
  const [streams, setStreams] = useState<StremioRawStream[]>([])
  const [streamError, setStreamError] = useState<string | null>(null)
  const [resolving, setResolving] = useState(false)
  const [listError, setListError] = useState<string | null>(null)
  const [searchQuery, setSearchQuery] = useState('')

  const refresh = useCallback(async () => {
    setLoading(true)
    setListError(null)
    try {
      const list = await listStremioAddons()
      console.log('[StremioAddonsView] listStremioAddons returned', list)
      setAddons(list)
      if (list.length > 0 && !list.find((a) => a.id === activeId)) {
        setActiveId(list[0].id)
      }
      if (list.length === 0) {
        setActiveId(null)
        setActiveCatalog(null)
        setMetas([])
      }
    } catch (e: any) {
      console.error('Failed to list Stremio addons', e)
      setListError(String(e?.message || e || 'Failed to list Stremio addons.'))
    } finally {
      setLoading(false)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  useEffect(() => {
    void refresh()
  }, [refresh])

  const active = useMemo(
    () => addons.find((a) => a.id === activeId) ?? null,
    [addons, activeId]
  )

  useEffect(() => {
    if (!active) {
      setActiveCatalog(null)
      setMetas([])
      return
    }
    if (!activeCatalog || activeCatalog.type === '' || !activeCatalog.id) {
      setActiveCatalog(active.catalogs[0] ?? null)
    }
  }, [active, activeCatalog])

  useEffect(() => {
    let cancelled = false
    if (!active || !activeCatalog) {
      setMetas([])
      return
    }
    setCatalogLoading(true)
    const extra: Record<string, string> = {}
    if (searchQuery.trim()) {
      extra.search = searchQuery.trim()
    }
    fetchStremioCatalog(active.id, activeCatalog.type, activeCatalog.id, extra)
      .then((r) => {
        if (cancelled) return
        setMetas(Array.isArray(r.metas) ? r.metas : [])
      })
      .catch((e) => {
        if (cancelled) return
        console.error('catalog fetch failed', e)
        setMetas([])
      })
      .finally(() => {
        if (cancelled) return
        setCatalogLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [active, activeCatalog])

  const onSelectMeta = useCallback(
    async (m: any) => {
      if (!active || !activeCatalog) return
      setActiveMeta(m)
      setStreamError(null)
      setStreams([])
      try {
        const r = await fetchStremioStreams(active.id, activeCatalog.type, m.id)
        setStreams(Array.isArray(r.streams) ? (r.streams as StremioRawStream[]) : [])
      } catch (e: any) {
        setStreamError(String(e?.message || e || 'Could not load streams.'))
      }
    },
    [active, activeCatalog]
  )

  const onPlay = useCallback(
    async (raw: StremioRawStream) => {
      if (!active) return
      let resolvedUrl: string | undefined = raw.url ?? undefined
      if (streamNeedsDebrid(raw)) {
        setResolving(true)
        try {
          const like: StremioStreamLike = {
            url: raw.url ?? null,
            infoHash: raw.infoHash ?? null,
            title: raw.title ?? raw.name ?? null,
            name: raw.name ?? null,
            ytId: raw.ytId ?? null,
          }
          resolvedUrl = await resolveStremioStream(like)
        } catch (e: any) {
          setStreamError(
            `Couldn't resolve this stream. Add a default debrid service in Settings → Debrid services. (${String(
              e?.message || e
            )})`
          )
          setResolving(false)
          return
        } finally {
          setResolving(false)
        }
      }
      const adapted = toRemoteStreamData(raw, {
        addonName: active.name,
        resolvedUrl: resolvedUrl ?? null,
      })
      if (adapted) onPlayStream(adapted)
    },
    [active, onPlayStream]
  )

  const onRemove = useCallback(
    async (id: string) => {
      try {
        await removeStremioAddon(id)
        if (id === activeId) {
          setActiveId(null)
          setActiveMeta(null)
          setStreams([])
        }
        await refresh()
      } catch (e) {
        console.error('remove failed', e)
      }
    },
    [activeId, refresh]
  )

  if (loading) {
    return (
      <div className="sv-stremio-loading">
        <Loader2 className="size-4 animate-spin" />
        <span>Loading Stremio addons…</span>
      </div>
    )
  }

  if (addons.length === 0) {
    return (
      <div className="sv-stremio-empty">
        <Plug className="size-5" aria-hidden="true" />
        <h3>{listError ? 'Could not load Stremio addons' : 'No Stremio addons yet'}</h3>
        <p>
          {listError
            ? listError
            : 'Paste a Stremio addon link in the setup card to get started. Stremio addons can provide catalogs, metadata, and streams — including via debrid services.'}
        </p>
      </div>
    )
  }

  return (
    <div className="sv-stremio-root">
      <div className="sv-stremio-tabs" role="tablist" aria-label="Stremio addons">
        {addons.map((a) => (
          <button
            key={a.id}
            role="tab"
            aria-selected={a.id === activeId}
            data-status={a.status}
            className="sv-stremio-tab"
            onClick={() => setActiveId(a.id)}
          >
            {a.logo ? (
              <img src={a.logo} alt="" className="sv-stremio-tab-logo" />
            ) : (
              <span className="sv-stremio-tab-mark" aria-hidden="true">
                <Plug className="size-3" />
              </span>
            )}
            <span className="sv-stremio-tab-name">{a.name}</span>
            <span
              className="sv-stremio-status-dot"
              data-status={a.status}
              aria-label={a.status}
            />
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation()
                void onRemove(a.id)
              }}
              aria-label={`Remove ${a.name}`}
              className="sv-stremio-tab-remove"
            >
              <Trash2 className="size-3" />
            </button>
          </button>
        ))}
      </div>

      {active && (
        <div className="sv-stremio-body">
          {active.status === 'unavailable' && (
            <div className="sv-stremio-banner" role="alert">
              <AlertCircle className="size-3.5" />
              <span>This addon is currently unavailable.</span>
              <button
                type="button"
                onClick={() => void refresh()}
                className="sv-stremio-banner-btn"
              >
                <RefreshCw className="size-3" />
                <span>Retry</span>
              </button>
            </div>
          )}

          {active.catalogs.length > 1 && (
            <div className="sv-stremio-catalogs" role="tablist" aria-label="Catalogs">
              {active.catalogs.map((c) => (
                <button
                  key={`${c.type}-${c.id}`}
                  role="tab"
                  aria-selected={
                    activeCatalog?.id === c.id && activeCatalog?.type === c.type
                  }
                  className="sv-stremio-catalog"
                  onClick={() => setActiveCatalog(c)}
                >
                  {c.name ?? c.id}
                </button>
              ))}
            </div>
          )}

          {/* Search bar */}
          <div className="px-8 pb-3">
            <div className="relative max-w-md">
              <Search className="absolute left-3 top-1/2 -translate-y-1/2 size-3.5 text-neutral-500" aria-hidden="true" />
              <input
                type="text"
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                placeholder={`Search ${active?.name ?? 'addons'}…`}
                className="w-full h-9 pl-9 pr-4 rounded-lg bg-neutral-900 border border-neutral-800 text-sm text-neutral-200 placeholder:text-neutral-600 focus:outline-none focus:border-neutral-600 transition-colors"
                aria-label="Search addons"
              />
            </div>
          </div>

          {catalogLoading ? (
            <div className="sv-stremio-loading">
              <Loader2 className="size-4 animate-spin" />
              <span>Loading…</span>
            </div>
          ) : metas.length === 0 ? (
            <div className="sv-stremio-empty">
              <p>No content available right now.</p>
            </div>
          ) : (
            <div className="sv-stremio-grid">
              {metas.map((m: any) => (
                <button
                  key={m.id}
                  className="sv-stremio-card"
                  onClick={() => void onSelectMeta(m)}
                >
                  {m.poster ? (
                    <img src={m.poster} alt="" className="sv-stremio-card-poster" />
                  ) : (
                    <div className="sv-stremio-card-poster sv-stremio-card-poster-fallback" />
                  )}
                  <div className="sv-stremio-card-meta">
                    <p className="sv-stremio-card-title">{m.name || m.title}</p>
                    {m.year && <p className="sv-stremio-card-sub">{m.year}</p>}
                  </div>
                </button>
              ))}
            </div>
          )}

          {activeMeta && (
            <div className="sv-stremio-streams">
              <h3>{activeMeta.name || activeMeta.title}</h3>
              {streams.length === 0 ? (
                <p className="sv-stremio-stream-empty">
                  {streamError ?? 'No streams available for this title.'}
                </p>
              ) : (
                <ul className="sv-stremio-stream-list">
                  {streams.map((s, idx) => {
                    const needs = streamNeedsDebrid(s)
                    return (
                      <li key={idx} className="sv-stremio-stream-row">
                        <div>
                          <p className="sv-stremio-stream-title">
                            {s.title || s.name || `Stream ${idx + 1}`}
                          </p>
                          {s.description && (
                            <p className="sv-stremio-stream-sub">{s.description}</p>
                          )}
                        </div>
                        <button
                          type="button"
                          disabled={resolving}
                          onClick={() => void onPlay(s)}
                          className="sv-stremio-stream-play"
                        >
                          {resolving ? (
                            <Loader2 className="size-3 animate-spin" />
                          ) : needs ? (
                            <span>Resolve &amp; Play</span>
                          ) : (
                            <span>Play</span>
                          )}
                        </button>
                      </li>
                    )
                  })}
                </ul>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  )
}

export default StremioAddonsView
