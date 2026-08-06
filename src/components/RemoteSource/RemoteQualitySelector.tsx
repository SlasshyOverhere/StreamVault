import { useState, useMemo, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { ScrollArea } from '@/components/ui/scroll-area'
import { useToast } from '@/components/ui/use-toast'
import { HardDrive, Film, Subtitles, AudioLines, Monitor, Database, Plus, Play, Copy, Check, WifiOff, Eye, EyeOff, Loader2, ExternalLink } from 'lucide-react'
import { formatFileSize } from './remote.types'
import { cn } from '@/lib/utils'
import type { GroupedStreams, RemoteStreamData, QualityFilter } from './remote.types'

interface ParsedMeta {
  codec: string | null
  hdr: string | null
  audio: string | null
  source: string | null
  hoster: string | null
  group: string | null
  size: string | null
}

function StreamMetaTags({ description, videoSize }: { description: string; videoSize: number }) {
  const meta = useMemo(() => parseStreamDescription(description), [description])
  const sizeLabel = videoSize > 0 ? formatFileSize(videoSize) : meta.size
  const tags: { key: string; label: string | null; icon: React.ComponentType<{ className?: string }> | null }[] = [
    ...(sizeLabel ? [{ key: 'size' as const, label: sizeLabel, icon: HardDrive }] : []),
    { key: 'source', label: meta.source, icon: Monitor },
    { key: 'codec', label: meta.codec, icon: Database },
    { key: 'hdr', label: meta.hdr, icon: Subtitles },
    { key: 'audio', label: meta.audio, icon: AudioLines },
    { key: 'hoster', label: meta.hoster, icon: null },
    { key: 'group', label: meta.group, icon: null },
  ].filter((t) => t.label)

  if (tags.length === 0) {
    return (
      <p className="text-[12px] text-neutral-600 leading-relaxed line-clamp-2">
        {description}
      </p>
    )
  }

  return (
    <div className="flex flex-wrap gap-1.5">
      {tags.map((t) => (
        <span
          key={t.key}
          className="inline-flex items-center gap-1 px-2 py-0.5 rounded-md text-[10px] font-semibold uppercase tracking-wider bg-neutral-800/60 text-neutral-400"
        >
          {t.icon && <t.icon className="size-3" />}
          {t.label}
        </span>
      ))}
    </div>
  )
}

function parseStreamDescription(desc: string): ParsedMeta {
  const m: ParsedMeta = { codec: null, hdr: null, audio: null, source: null, hoster: null, group: null, size: null }

  const codecs = desc.match(/x265|x264|HEVC|AVC|AV1|VP9|MPEG-2/i)
  if (codecs) m.codec = codecs[0].toUpperCase()

  const hdrs = desc.match(/\b(SDR|HDR10?|Dolby\s*Vision|HLG|HDR)\b/i)
  if (hdrs) m.hdr = hdrs[1] || hdrs[0]

  const audios = desc.match(/(?:DDP?|TrueHD|FLAC|AAC|AC3|E-?AC-?3|DTS(?:\s*[-\s]?HD)?)\s*[\d.]+\s*ch/i)
  if (audios) m.audio = audios[0]

  const sources = desc.match(/\b(WEB-?DL|BluRay|WEBRip|BRRip|DVDRip|HDTV|HDRip|CAM|TS|TC)\b/i)
  if (sources) m.source = sources[1] || sources[0]

  const hosters = desc.match(/\|\s*([A-Za-z0-9]+)\s*$/)
  if (hosters) m.hoster = hosters[1]

  const groups = desc.match(/^\[([^\]]+)\]/)
  if (groups) m.group = groups[1]

  const sizes = desc.match(/(?:💾\s*)?([\d.]+)\s*(GB|GiB|MB|MiB)\b/i)
  if (sizes) m.size = `${sizes[1]} ${sizes[2].toUpperCase()}`

  return m
}

const QUALITY_FILTERS: QualityFilter[] = ['all', '4K', '1080p', '720p']

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  groupedStreams: GroupedStreams[]
  onSelect: (stream: RemoteStreamData) => void
  onOpenUrl?: (url: string) => void
  loading?: boolean
  error?: string | null
  verifying?: boolean
  streamStatus?: Record<string, boolean>
  /** Set of URLs currently being verified — only these show checking/active/inactive badges */
  verifyingUrls?: Set<string>
  /** If set, shows "Index to Direct Links" button on season pack streams */
  addonContext?: { imdbId: string; season: number } | null
}

export function RemoteQualitySelector({
  open, onOpenChange, title, groupedStreams, onSelect, onOpenUrl, loading, error, verifying, streamStatus = {},
  verifyingUrls = new Set(), addonContext = null,
}: Props) {
  const { toast } = useToast()
  const [qualityFilter, setQualityFilter] = useState<QualityFilter>('all')
  const [copiedStreamUrl, setCopiedStreamUrl] = useState<string | null>(null)
  const [showInactive, setShowInactive] = useState(false)
  // HubDrive validation: url -> { valid: boolean, title: string } | null (null = pending)
  const [hubdriveStatus, setHubdriveStatus] = useState<Record<string, { valid: boolean; title: string } | 'loading'>>({})
  const [indexingStream, setIndexingStream] = useState<string | null>(null)
  const [indexedStreams, setIndexedStreams] = useState<Set<string>>(new Set())
  const [indexProgress, setIndexProgress] = useState<string | null>(null)

  const handleIndexToDdl = async (stream: RemoteStreamData) => {
    if (!addonContext) return
    onOpenChange(false)
    setIndexingStream(stream.url)
    setIndexProgress('Validating stream...')
    const unlisten = await listen<{ stage: string; message: string; filename?: string }>('ddl-index-progress', (event) => {
      setIndexProgress(event.payload.message)
    })
    try {
      await invoke('index_season_pack_to_ddl', {
        url: stream.url,
        imdbId: addonContext.imdbId,
        seasonNumber: addonContext.season,
        streamName: stream.name,
      })
      setIndexedStreams(prev => new Set(prev).add(stream.url))
      setIndexProgress('Done!')
      window.dispatchEvent(new CustomEvent('navigate-to-ddl'))
    } catch (e: any) {
      setIndexProgress(null)
      toast({ title: 'Indexing failed', description: String(e), variant: 'destructive' })
    } finally {
      unlisten()
      setIndexingStream(null)
      setTimeout(() => setIndexProgress(null), 1000)
    }
  }

  const verificationDone = !verifying && verifyingUrls.size > 0 && Object.keys(streamStatus).length > 0

  const totalUrls = useMemo(() => {
    const urls = new Set<string>()
    for (const g of groupedStreams) {
      for (const s of g.streams) {
        urls.add(s.url)
      }
    }
    return urls.size
  }, [groupedStreams])

  const verifiedCount = useMemo(() => {
    return Object.keys(streamStatus).length
  }, [streamStatus])

  const inactiveCount = useMemo(() => {
    let count = 0
    for (const url of verifyingUrls) {
      if (streamStatus[url] === false) count++
    }
    return count
  }, [verifyingUrls, streamStatus])

  const isStreamActive = (url: string): boolean => {
    if (!verifyingUrls.has(url)) return true
    return streamStatus[url] !== false
  }

  const [showHubdrive, setShowHubdrive] = useState(false)
  const hubdriveStreams = useMemo(() => {
    const hd: RemoteStreamData[] = []
    for (const g of groupedStreams) {
      if (qualityFilter !== 'all' && g.quality !== qualityFilter) continue
      for (const s of g.streams) {
        if (s.isHubdrive) hd.push(s)
      }
    }
    return hd
  }, [groupedStreams, qualityFilter])

  // Validate hubdrive URLs when section is expanded
  useEffect(() => {
    if (!showHubdrive || hubdriveStreams.length === 0) return
    for (const stream of hubdriveStreams) {
      if (hubdriveStatus[stream.url]) continue
      setHubdriveStatus(prev => ({ ...prev, [stream.url]: 'loading' }))
      invoke<{ isValid: boolean; title: string }>('validate_hubdrive_url', { url: stream.url })
        .then(res => setHubdriveStatus(prev => ({ ...prev, [stream.url]: { valid: res.isValid, title: res.title } })))
        .catch(() => setHubdriveStatus(prev => ({ ...prev, [stream.url]: { valid: false, title: 'Validation failed' } })))
    }
  }, [showHubdrive, hubdriveStreams])

  const filtered = useMemo(() => {
    let groups = groupedStreams
    if (qualityFilter !== 'all') {
      groups = groups.filter((g) => g.quality === qualityFilter)
    }
    // Exclude hubdrive from regular list
    groups = groups
      .map((g) => ({
        ...g,
        streams: g.streams.filter((s) => !s.isHubdrive),
      }))
      .filter((g) => g.streams.length > 0)
    if (!showInactive && verificationDone) {
      groups = groups
        .map((g) => ({
          ...g,
          streams: g.streams.filter((s) => streamStatus[s.url] !== false),
        }))
        .filter((g) => g.streams.length > 0)
    }
    return groups
  }, [groupedStreams, qualityFilter, showInactive, verificationDone, streamStatus])

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl bg-[#0A0A0A] border-neutral-800 text-neutral-100 shadow-2xl">
        <DialogHeader>
          <DialogTitle className="text-lg font-semibold text-neutral-100 flex items-center gap-2">
            <Film className="size-4 text-amber-500/70" />
            <span>{title}</span>
          </DialogTitle>
        </DialogHeader>

        {loading && (
          <div className="flex items-center justify-center py-12 gap-3">
            <div className="size-5 rounded-full border-2 border-neutral-700 border-t-amber-600/60 animate-spin" />
            <span className="text-sm text-neutral-500 font-medium">Loading available streams...</span>
          </div>
        )}

        {error && (
          <div className="text-sm text-red-400 bg-red-500/5 border border-red-800/30 rounded-xl p-4 font-medium">
            {error}
          </div>
        )}

        {!loading && !error && groupedStreams.length > 0 && (
          <>
            {/* Verification banner */}
            {verifying && (
              <div className="flex items-center gap-3 px-4 py-3 rounded-xl bg-amber-500/5 border border-amber-700/20">
                <Loader2 className="size-4 text-amber-400 animate-spin shrink-0" />
                <div className="flex-1 min-w-0">
                  <p className="text-sm font-semibold text-amber-300/90">Checking Pixeldrain links...</p>
                  <p className="text-[11px] text-neutral-500 font-medium">
                    {verifiedCount} of {verifyingUrls.size} checked &middot; {inactiveCount} expired
                  </p>
                </div>
              </div>
            )}

            {/* Quality filter chips */}
            <div className="flex gap-1.5" role="radiogroup" aria-label="Video quality filter">
              {QUALITY_FILTERS.map((f) => (
                <button
                  key={f}
                  role="radio"
                  aria-checked={qualityFilter === f}
                  aria-label={f === 'all' ? 'All qualities' : f}
                  onClick={() => setQualityFilter(f)}
                  className={cn(
                    'px-3.5 py-1.5 rounded-xl text-xs font-semibold uppercase tracking-wider transition-all duration-200 border',
                    qualityFilter === f
                      ? 'bg-amber-600/15 text-amber-400 border-amber-700/30'
                      : 'bg-[#0D0D0D] text-neutral-500 border-neutral-800 hover:bg-neutral-900 hover:text-neutral-300 hover:border-neutral-700',
                  )}
                >
                  {f === 'all' ? 'All' : f}
                </button>
              ))}
            </div>

            {/* Show inactive toggle */}
            {verificationDone && inactiveCount > 0 && (
              <button
                onClick={() => setShowInactive(!showInactive)}
                aria-pressed={showInactive}
                aria-label={showInactive ? 'Hide inactive sources' : `Show inactive sources (${inactiveCount})`}
                className="flex items-center gap-2 text-[11px] font-semibold uppercase tracking-wider text-neutral-600 hover:text-neutral-300 transition-colors"
              >
                {showInactive ? <EyeOff className="size-3.5" /> : <Eye className="size-3.5" />}
                {showInactive ? `Hide inactive sources` : `Show inactive sources (${inactiveCount})`}
              </button>
            )}

            {filtered.length === 0 && hubdriveStreams.length === 0 && (
              <div className="text-sm text-neutral-600 text-center py-10 font-medium">
                {verificationDone && inactiveCount === totalUrls
                  ? 'No active streams available. Toggle "Show inactive sources" above to try anyway.'
                  : `No ${qualityFilter} streams available`}
              </div>
            )}

            {hubdriveStreams.length > 0 && (
              <div className="mt-3 pt-3 border-t border-neutral-800/50">
                <button
                  onClick={() => setShowHubdrive(!showHubdrive)}
                  className="w-full text-left text-[11px] font-bold text-neutral-500 uppercase tracking-widest mb-2.5 px-1 flex items-center gap-2 hover:text-neutral-400 transition-colors"
                >
                  <ExternalLink className="size-3.5" />
                  HubDrive (Login Required) — {hubdriveStreams.length}
                  <span className="ml-auto text-[10px]">{showHubdrive ? '▲' : '▼'}</span>
                </button>
                {showHubdrive && (<>
                <p className="text-[10px] text-neutral-600 mb-2 px-1">These require logging in on the hosting site. Opens in your browser.</p>

                <div className="space-y-2">
                  {hubdriveStreams.map((stream) => {
                    const status = hubdriveStatus[stream.url]
                    const isValid = status && status !== 'loading' ? status.valid : null
                    const isLoading = status === 'loading'
                    return (
                      <button
                        key={stream.url}
                        onClick={() => onOpenUrl?.(stream.url)}
                        className="w-full text-left p-3 rounded-xl bg-[#0D0D0D] border border-neutral-800 hover:bg-neutral-900 hover:border-neutral-700/60 transition-all duration-200 group"
                      >
                        <div className="flex items-center justify-between gap-3">
                          <span className="text-sm font-medium text-neutral-300 truncate">{stream.name}</span>
                          <span className="shrink-0 flex items-center gap-1.5">
                            {isLoading && <Loader2 className="size-3 animate-spin text-neutral-500" />}
                            {isValid === true && <span className="text-[10px] font-bold text-emerald-400">Active</span>}
                            {isValid === false && <span className="text-[10px] font-bold text-red-400">Expired</span>}
                            <span className="flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-[10px] font-bold bg-blue-600/10 text-blue-400/80 border border-blue-700/20">
                              <ExternalLink className="size-3" />
                              Open
                            </span>
                          </span>
                        </div>
                        {stream.description && (
                          <div className="mt-1.5 border-t border-neutral-800/50 pt-1.5">
                            <StreamMetaTags description={stream.description} videoSize={stream.videoSize} />
                          </div>
                        )}
                      </button>
                    )
                  })}
                </div>
                </>
                )}
              </div>
            )}

            {filtered.length > 0 && (
              <ScrollArea className="max-h-[420px]">
                <div className="space-y-3 pr-3" role="listbox" aria-label="Select video stream">
                  {filtered.map((group) => (
                    <div key={group.quality}>
                      <h4 className="text-[11px] font-bold text-neutral-600 uppercase tracking-widest mb-2.5 px-1">
                        {group.quality}
                      </h4>
                      <div className="space-y-2">
                        {group.streams.map((stream) => {
                          const active = isStreamActive(stream.url)
                          const checkPending = verifyingUrls.has(stream.url) && streamStatus[stream.url] === undefined

                          return (
                            <button
                              key={stream.url}
                              role="option"
                              aria-selected={false}
                              aria-label={`${stream.name}${!active ? ' (unreachable)' : ''}`}
                              onClick={() => active && onSelect(stream)}
                              className={cn(
                                'w-full text-left p-4 rounded-2xl bg-[#0D0D0D] border group transition-all duration-200',
                                active
                                  ? 'border-neutral-800 hover:bg-neutral-900 hover:border-neutral-700/60'
                                  : 'border-neutral-800/40 opacity-50 cursor-default',
                              )}
                            >
                              <div className="flex items-start justify-between gap-4">
                                <div className="flex items-center gap-3 min-w-0 flex-1">
                                  <span className={cn(
                                    'text-sm font-semibold truncate',
                                    active ? 'text-neutral-200' : 'text-neutral-500',
                                  )}>
                                    {stream.name}
                                  </span>

                                  {/* Active badge */}
                                  {verificationDone && verifyingUrls.has(stream.url) && active && (
                                    <span className="shrink-0 flex items-center gap-1 px-2 py-0.5 rounded-lg text-[10px] font-bold bg-green-500/10 text-green-500/70 border border-green-700/20">
                                      Active
                                    </span>
                                  )}

                                  {/* Inactive badge */}
                                  {verificationDone && verifyingUrls.has(stream.url) && !active && (
                                    <span className="shrink-0 flex items-center gap-1 px-2 py-0.5 rounded-lg text-[10px] font-bold bg-red-500/10 text-red-400/70 border border-red-800/20">
                                      <WifiOff className="size-3" />
                                      {/pixeldrain\.\w+/i.test(stream.url) ? 'Expired' : 'Unreachable'}
                                    </span>
                                  )}

                                  {/* Checking badge */}
                                  {checkPending && (
                                    <span className="shrink-0 flex items-center gap-1 px-2 py-0.5 rounded-lg text-[10px] font-bold bg-neutral-800/60 text-neutral-500 border border-neutral-700/30">
                                      <Loader2 className="size-3 animate-spin" />
                                      Checking...
                                    </span>
                                  )}

                                </div>

                                <div className="flex items-center gap-2 shrink-0">
                                  {addonContext && (
                                    <div
                                      role="button"
                                      tabIndex={0}
                                      aria-label={indexedStreams.has(stream.url) ? "Indexed to Direct Links" : "Index to Direct Links"}
                                      onClick={(e) => { e.stopPropagation(); if (!indexedStreams.has(stream.url) && !indexingStream) handleIndexToDdl(stream) }}
                                      onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); e.stopPropagation(); if (!indexedStreams.has(stream.url) && !indexingStream) handleIndexToDdl(stream) } }}
                                      className={cn(
                                        'size-9 flex items-center justify-center rounded-xl transition-all duration-200 cursor-pointer',
                                        indexedStreams.has(stream.url)
                                          ? 'bg-emerald-600/10 text-emerald-500'
                                          : indexingStream === stream.url
                                            ? 'bg-neutral-800/50 text-neutral-400'
                                            : 'bg-neutral-800/50 text-neutral-500 hover:bg-neutral-700/50 hover:text-neutral-300'
                                      )}
                                    >
                                      {indexedStreams.has(stream.url) ? <Check className="size-4" /> : indexingStream === stream.url ? <Loader2 className="size-4 animate-spin" /> : <Plus className="size-4" />}
                                    </div>
                                  )}
                                  <div
                                    role="button"
                                    tabIndex={0}
                                    aria-label="Copy stream URL"
                                    onClick={(e) => { e.stopPropagation(); navigator.clipboard.writeText(stream.url).then(() => { setCopiedStreamUrl(stream.url); setTimeout(() => setCopiedStreamUrl(null), 2000) }).catch(() => { /* clipboard unavailable */ }) }}
                                    onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); e.stopPropagation(); navigator.clipboard.writeText(stream.url).then(() => { setCopiedStreamUrl(stream.url); setTimeout(() => setCopiedStreamUrl(null), 2000) }).catch(() => { /* clipboard unavailable */ }) } }}
                                    className="size-9 flex items-center justify-center rounded-xl bg-neutral-800/50 text-neutral-500 hover:bg-neutral-700/50 hover:text-neutral-300 transition-all duration-200 cursor-pointer"
                                  >
                                    {copiedStreamUrl === stream.url ? <Check className="size-4 text-emerald-400" /> : <Copy className="size-4" />}
                                  </div>
                                  <div className={cn(
                                    'size-9 flex items-center justify-center rounded-xl transition-all duration-200',
                                    active
                                      ? 'bg-amber-600/10 text-amber-500 group-hover:bg-amber-600/20 group-hover:text-amber-400'
                                      : 'bg-neutral-800/30 text-neutral-600',
                                  )}>
                                    <Play className="size-4" />
                                  </div>
                                </div>
                              </div>

                              {stream.description && (
                                <div className="mt-1.5 border-t border-neutral-800/50 pt-1.5">
                                  <StreamMetaTags description={stream.description} videoSize={stream.videoSize} />
                                </div>
                              )}
                            </button>
                          )
                        })}
                      </div>
                    </div>
                  ))}
                </div>
              </ScrollArea>
            )}
          </>
        )}

        {!loading && !error && groupedStreams.length === 0 && (
          <div className="text-sm text-neutral-600 text-center py-10 font-medium">
            No streams available
          </div>
        )}
      </DialogContent>

      {/* Indexing progress overlay */}
      {indexProgress && (
        <div className="fixed inset-0 z-[200] flex items-center justify-center bg-black/60 backdrop-blur-sm">
          <div className="flex flex-col items-center gap-4 px-8 py-6 rounded-2xl bg-[#0D0D0D] border border-neutral-800">
            <div className="size-8 rounded-full border-2 border-neutral-700 border-t-amber-600/60 animate-spin" />
            <div className="text-sm text-neutral-300 font-medium text-center max-w-[280px]">{indexProgress}</div>
          </div>
        </div>
      )}
    </Dialog>
  )
}
