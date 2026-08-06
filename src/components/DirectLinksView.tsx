import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { LazyMotion, m, domAnimation, AnimatePresence } from "framer-motion"
import {
  Link2, Plus, Trash2, RefreshCw,
  AlertCircle, CheckCircle, Loader2, Archive,
  Sparkles,
  HardDrive, ChevronDown, Globe, CornerDownLeft, FileArchive
} from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog"
import { cn } from "@/lib/utils"
import { formatFileSize } from "@/utils/format"
import { useToast } from "@/components/ui/use-toast"
import type { MediaItem as ApiMediaItem } from "@/services/api"
import { DdlMediaLibrary } from "./DdlMediaLibrary"

interface DdlSource {
  id: string
  url: string
  filename: string
  fileSize: number
  archiveFormat: string
  entryCount: number
  videoCount: number
  cdOffset: number
  cdSize: number
  createdAt: string
  lastVerifiedAt: string
  isExpired: boolean
}

interface DdlValidationResult {
  supportsRange: boolean
  fileSize: number
  filename: string
  contentType: string
}

interface DdlRefreshResult {
  accepted: boolean
  message: string
}

interface MediaItem {
  id: number
  title: string
  media_type: string
  season_number?: number
  episode_number?: number
  zip_entry_path?: string
  zip_uncompressed_size?: number
  file_path?: string
}

interface DdlIndexProgressPayload {
  stage: string
  message: string
  filename?: string | null
  current?: number | null
  total?: number | null
  season?: number | null
  episode?: number | null
  episodeTitle?: string | null
}

function formatSeasonEpisode(season?: number | null, episode?: number | null): string | null {
  if (season == null && episode == null) return null
  if (season != null && episode != null) {
    return `S${String(season).padStart(2, "0")} E${String(episode).padStart(2, "0")}`
  }
  if (season != null) return `Season ${season}`
  return `Episode ${episode}`
}

type Step = "idle" | "validating" | "indexing" | "done" | "error"

function prettifyFilename(filename: string): { display: string; extension: string } {
  const dotIndex = filename.lastIndexOf(".")
  const extension = dotIndex > 0 ? filename.slice(dotIndex) : ""
  const nameWithoutExt = dotIndex > 0 ? filename.slice(0, dotIndex) : filename
  const display = nameWithoutExt.replace(/\./g, " ")
  return { display, extension }
}

function timeAgo(dateStr: string): string {
  const date = new Date(dateStr + "Z")
  const now = new Date()
  const diff = Math.floor((now.getTime() - date.getTime()) / 1000)
  if (diff < 60) return "just now"
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`
  return `${Math.floor(diff / 86400)}d ago`
}

function parseArchiveUrl(raw: string): { host: string | null; filename: string | null; ext: string | null } {
  const url = raw.trim()
  if (!url) return { host: null, filename: null, ext: null }
  try {
    const u = new URL(url)
    if (u.protocol !== "http:" && u.protocol !== "https:") return { host: null, filename: null, ext: null }
    const pathname = u.pathname.split("/").filter(Boolean).pop() ?? null
    let filename: string | null = null
    let ext: string | null = null
    if (pathname) {
      const dot = pathname.lastIndexOf(".")
      if (dot > 0) {
        filename = pathname
        ext = pathname.slice(dot).toLowerCase()
      } else {
        filename = pathname
        ext = null
      }
    }
    return { host: u.host, filename, ext }
  } catch {
    return { host: null, filename: null, ext: null }
  }
}

interface ParsedError {
  status: string | null
  statusText: string | null
  uri: string | null
  raw: string
}

function parseIndexError(raw: string): ParsedError {
  // Match patterns like: HTTP status client error (403 Forbidden) for uri (https://...)
  const statusMatch = raw.match(/\((\d{3})(?:\s+([A-Za-z][\w\s]*?))?\)/)
  const uriMatch = raw.match(/for uri\s+\(([^)]+)\)/i) ?? raw.match(/(https?:\/\/[^\s)]+)/i)
  return {
    status: statusMatch?.[1] ?? null,
    statusText: statusMatch?.[2]?.trim() ?? null,
    uri: uriMatch?.[1] ?? null,
    raw,
  }
}

function errorRecoveryHint(parsed: ParsedError): string {
  if (!parsed.status) return "Check the URL, then try again."
  const code = Number(parsed.status)
  if (code === 401 || code === 403) {
    return "The link is private or expired. Get a fresh URL from the host and try again."
  }
  if (code === 404) {
    return "The file is no longer at this address. Check the URL or find a new source."
  }
  if (code === 408 || code === 504 || code === 524) {
    return "The host didn't respond in time. Try again, or pick a faster mirror."
  }
  if (code >= 500) {
    return "The host is having trouble. Wait a moment, then try again."
  }
  if (code >= 400) {
    return "The host rejected the request. Verify the URL is correct and still valid."
  }
  return "Check the URL, then try again."
}

function shortenUriForDisplay(uri: string, maxLen: number = 64): string {
  try {
    const u = new URL(uri)
    const last = u.pathname.split("/").filter(Boolean).pop() ?? ""
    const base = `${u.host}/…/${last}`
    if (base.length <= maxLen) return base
    return base.slice(0, maxLen - 1) + "…"
  } catch {
    if (uri.length <= maxLen) return uri
    return uri.slice(0, maxLen - 1) + "…"
  }
}

interface DirectLinksViewProps {
  onIndexComplete?: (payload: { mediaIds: number[]; contentName: string }) => void | Promise<void>
  viewMode?: "grid" | "list"
  onItemClick?: (item: ApiMediaItem) => void
  onFixMatch?: (item: ApiMediaItem) => void
  onDownload?: (item: ApiMediaItem) => void | Promise<void>
  onDelete?: (item: ApiMediaItem) => void
  onWatchTogether?: (item: ApiMediaItem) => void
}

export default function DirectLinksView({
  onIndexComplete,
  viewMode = "grid",
  onItemClick,
  onFixMatch,
  onDownload,
  onDelete,
  onWatchTogether,
}: DirectLinksViewProps) {
  const { toast } = useToast()
  const [sources, setSources] = useState<DdlSource[]>([])
  const [loading, setLoading] = useState(true)
  const [showAddModal, setShowAddModal] = useState(false)
  const [sourceMedia, setSourceMedia] = useState<Record<string, MediaItem[]>>({})
  const [refreshModal, setRefreshModal] = useState<string | null>(null)
  const [refreshUrl, setRefreshUrl] = useState("")
  const [refreshing, setRefreshing] = useState(false)
  const [refreshError, setRefreshError] = useState("")
  const [checkingHealth, setCheckingHealth] = useState<string | null>(null)
  const [isSourcesDropdownOpen, setIsSourcesDropdownOpen] = useState(false)
  const [mediaRefreshKey, setMediaRefreshKey] = useState(0)

  const [addUrl, setAddUrl] = useState("")
  const [addStep, setAddStep] = useState<Step>("idle")
  const [addError, setAddError] = useState("")
  const [addValidation, setAddValidation] = useState<DdlValidationResult | null>(null)
  const [addProgress, setAddProgress] = useState<DdlIndexProgressPayload | null>(null)
  const [indexingTick, setIndexingTick] = useState(0)

  const progressCurrent = addProgress?.current ?? 0
  const progressTotal = addProgress?.total ?? 0
  const isIndeterminateProgress = addStep === "indexing" && (
    addProgress?.stage === "probing-archive" ||
    addProgress?.stage === "fetching-show-metadata"
  )
  const progressPercent = progressTotal > 0 ? Math.min(100, (progressCurrent / progressTotal) * 100) : 0
  const progressContext = formatSeasonEpisode(addProgress?.season, addProgress?.episode)
  const progressDots = addStep === "indexing" ? ".".repeat((indexingTick % 4)) : ""
  const progressMessage = addProgress?.message
    ? `${addProgress.message.replace(/\.+$/, "")}${progressDots}`
    : `Analyzing Headers${progressDots}`
  const progressLabel = addProgress?.stage === "fetching-episode-metadata"
    ? "TMDB Episode Metadata"
    : addProgress?.stage === "fetching-show-metadata"
      ? "TMDB Show Match"
      : addProgress?.stage === "archive-indexed"
        ? "Archive Analysis"
        : addProgress?.stage === "adding-entry"
          ? "Library Mapping"
          : addProgress?.stage === "probing-archive"
            ? "Archive Probe"
            : "Indexing"

  const fetchSources = useCallback(async () => {
    try {
      const result = await invoke<DdlSource[]>("ddl_get_sources")
      setSources(result)
    } catch (err: unknown) {
      console.error("Failed to fetch DDL sources:", err)
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => { fetchSources() }, [fetchSources])

  useEffect(() => {
    let unlisten: (() => void) | undefined
    void (async () => {
      unlisten = await listen<DdlIndexProgressPayload>("ddl-index-progress", (event) => {
        setAddProgress(event.payload)
      })
    })()
    return () => { unlisten?.() }
  }, [])

  useEffect(() => {
    if (addStep !== "indexing") { setIndexingTick(0); return }
    const interval = window.setInterval(() => setIndexingTick(c => c + 1), 250)
    return () => window.clearInterval(interval)
  }, [addStep])

  useEffect(() => {
    const missingSourceIds = sources
      .map(s => s.id)
      .filter(id => sourceMedia[id] == null)
    if (missingSourceIds.length === 0) return
    let cancelled = false
    void Promise.all(
      missingSourceIds.map(async (sourceId) => {
        try {
          const media = await invoke<MediaItem[]>("ddl_get_source_media", { sourceId })
          if (cancelled) return
          setSourceMedia(current => {
            if (current[sourceId] != null) return current
            return { ...current, [sourceId]: media }
          })
        } catch (e) {
          console.warn('[DirectLinks] Failed to load source media:', e)
        }
      })
    )
    return () => { cancelled = true }
  }, [sources, sourceMedia])

  const handleAdd = async () => {
    if (!addUrl.trim()) return
    setAddStep("validating")
    setAddError("")
    setAddProgress(null)
    try {
      const validation = await invoke<DdlValidationResult>("ddl_validate_url", { url: addUrl.trim() })
      setAddValidation(validation)
      setAddStep("indexing")
      const indexedSource = await invoke<DdlSource>("ddl_index_archive", { url: addUrl.trim(), validation, addonOrigin: null })
      setAddStep("done")
      await fetchSources()
      let indexedMediaIds: number[] = []
      try {
        const indexedMedia = await invoke<MediaItem[]>("ddl_get_source_media", { sourceId: indexedSource.id })
        setSourceMedia(current => ({ ...current, [indexedSource.id]: indexedMedia }))
        indexedMediaIds = indexedMedia.reduce<number[]>((ids, m) => {
          if (m.media_type !== "tvshow") ids.push(m.id)
          return ids
        }, [])
      } catch (e) {
        console.warn('[DirectLinks] Failed to load indexed media:', e)
      }
      setTimeout(() => {
        setShowAddModal(false)
        setAddUrl("")
        setAddStep("idle")
        setAddValidation(null)
        setAddProgress(null)
        void onIndexComplete?.({ mediaIds: indexedMediaIds, contentName: indexedSource.filename })
      }, 2000)
    } catch (err: unknown) {
      setAddError(err instanceof Error ? err.message : String(err))
      setAddStep("error")
    }
  }

  const handleDelete = async (sourceId: string) => {
    try {
      await invoke("ddl_delete_source", { sourceId })
      setSources(s => s.filter(src => src.id !== sourceId))
      setSourceMedia(m => { const n = { ...m }; delete n[sourceId]; return n })
      setMediaRefreshKey(k => k + 1)
    } catch (err: unknown) {
      console.error("Failed to delete source:", err)
    }
  }

  const handleRefresh = async (sourceId: string) => {
    if (!refreshUrl.trim()) return
    setRefreshing(true)
    setRefreshError("")
    try {
      const result = await invoke<DdlRefreshResult>("ddl_refresh_link", { sourceId, newUrl: refreshUrl.trim() })
      if (result.accepted) {
        setRefreshModal(null)
        setRefreshUrl("")
        await fetchSources()
      } else {
        setRefreshError(result.message)
      }
    } catch (err: unknown) {
      setRefreshError(err instanceof Error ? err.message : String(err))
    } finally {
      setRefreshing(false)
    }
  }

  const handleCheckHealth = async (sourceId: string) => {
    setCheckingHealth(sourceId)
    try {
      const healthy = await invoke<boolean>("ddl_check_link_health", { sourceId })
      setSources(prev => prev.map(s => s.id === sourceId ? { ...s, lastVerifiedAt: new Date().toISOString().replace("Z", "") } : s))
      toast({
        title: healthy ? "Link healthy" : "Link expired",
        description: healthy ? "This source is still reachable." : "This source no longer responds.",
        variant: healthy ? "default" : "destructive",
      })
    } catch (err: unknown) {
      toast({ title: "Health check failed", description: String(err), variant: "destructive" })
    } finally {
      setCheckingHealth(null)
    }
  }

  return (
    <LazyMotion features={domAnimation}>
    <div className="flex flex-col relative h-full">

      <div className="px-8 py-12 relative z-10 flex flex-col h-full">
        {/* Header — CTA only renders when there are sources; the empty state carries its own action */}
        <div className="flex flex-col md:flex-row md:items-end justify-between gap-6 mb-8">
          <div>
            <h1 className="text-4xl font-black text-foreground tracking-tight mb-2">
              Direct Links
            </h1>
            <p className="text-sm text-muted-foreground max-w-md leading-relaxed">
              Stream straight from hosted ZIP &amp; RAR archives. Range-aware seeking, no full download.
            </p>
          </div>

          {sources.length > 0 && (
            <Button onClick={() => { setShowAddModal(true); setAddStep("idle"); setAddUrl(""); setAddError(""); setAddProgress(null); setAddValidation(null) }}>
              <Plus className="size-4 mr-2" />
              Add New Archive
            </Button>
          )}
        </div>

        {/* Sources Dropdown */}
        {loading ? (
          <div className="space-y-3">
            {[1, 2, 3].map((skeletonIdx) => (
              <div key={skeletonIdx} className="rounded-xl bg-card border border-border p-5 animate-pulse">
                <div className="flex items-center gap-4">
                  <div className="size-12 rounded-xl bg-muted" />
                  <div className="flex-1 space-y-2">
                    <div className="h-5 w-3/5 rounded-lg bg-muted" />
                    <div className="h-3 w-2/5 rounded-lg bg-muted/60" />
                  </div>
                  <div className="flex gap-2">
                    <div className="size-9 rounded-lg bg-muted" />
                    <div className="size-9 rounded-lg bg-muted" />
                  </div>
                </div>
              </div>
            ))}
          </div>
        ) : sources.length === 0 ? (
          <m.div
            initial={{ opacity: 0, y: 16 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.4, ease: [0.16, 1, 0.3, 1] }}
            className="relative overflow-hidden rounded-2xl border border-border bg-card"
          >
            {/* Top scanline — quiet signal that this is a tool surface, not a marketing card */}
            <div className="absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-white/15 to-transparent" />

            <div className="grid grid-cols-1 lg:grid-cols-[1fr_auto] gap-10 lg:gap-14 p-8 sm:p-10">
              {/* Left: copy + primary action + format row */}
              <div className="flex flex-col">
                <div className="flex items-center gap-2.5 mb-5">
                  <div className="size-8 rounded-lg bg-foreground flex items-center justify-center">
                    <Link2 className="size-4 text-background" strokeWidth={2.5} />
                  </div>
                  <span className="text-xs font-semibold text-foreground">Stream Engine</span>
                  <span className="size-1 rounded-full bg-border" aria-hidden />
                  <span className="text-xs text-muted-foreground">No sources yet</span>
                </div>

                <h2 className="text-[28px] sm:text-[32px] font-bold text-foreground leading-[1.1] tracking-[-0.02em] text-balance mb-3">
                  Point a hosted archive at MPV.
                </h2>
                <p className="text-sm text-muted-foreground max-w-md leading-relaxed mb-7">
                  Paste a direct link to a ZIP or RAR. SlasshyVault probes the headers, indexes the entries, and streams any file inside — seeking included, download skipped.
                </p>

                <div className="flex flex-wrap items-center gap-3">
                  <Button
                    size="lg"
                    onClick={() => { setShowAddModal(true); setAddStep("idle"); setAddUrl(""); setAddError(""); setAddProgress(null); setAddValidation(null) }}
                    className="min-w-[180px]"
                  >
                    <Plus className="size-4 mr-2" strokeWidth={2.5} />
                    Add an archive
                  </Button>
                </div>

                {/* Format row — real signal, not a promise */}
                <div className="mt-8 flex items-center gap-2">
                  <span className="text-[10px] font-medium text-muted-foreground uppercase tracking-wider">Supports</span>
                  {(["ZIP", "RAR"] as const).map((fmt) => (
                    <span
                      key={fmt}
                      className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-md bg-muted border border-border text-[11px] font-semibold text-foreground"
                    >
                      <span className="size-1 rounded-full bg-emerald-500" aria-hidden />
                      {fmt}
                    </span>
                  ))}
                  <span className="text-[11px] text-muted-foreground ml-1">Range-required</span>
                </div>
              </div>

              {/* Right: ghost preview of what an indexed source looks like */}
              <div className="relative w-full lg:w-[340px] shrink-0">
                <div className="text-[10px] font-medium text-muted-foreground uppercase tracking-wider mb-3">
                  What you'll see
                </div>
                <div className="space-y-2">
                  {[
                    { name: "Series.S01.2160p.WEB-DL", ext: ".rar", size: "14.8 GB", videos: 10, dot: "emerald" },
                    { name: "Backlog.2024.Movies.Pack", ext: ".zip", size: "62.3 GB", videos: 48, dot: "emerald" },
                    { name: "Documentary.Archive.2003", ext: ".rar", size: "8.1 GB", videos: 6, dot: "muted" },
                  ].map((row, i) => (
                    <m.div
                      key={row.name}
                      initial={{ opacity: 0, x: 8 }}
                      animate={{ opacity: 1, x: 0 }}
                      transition={{ duration: 0.3, delay: 0.15 + i * 0.06, ease: "easeOut" }}
                      className="flex items-center gap-3 p-3 rounded-xl bg-muted/30 border border-border/60"
                    >
                      <div className="size-9 rounded-lg bg-muted border border-border flex items-center justify-center shrink-0">
                        <Archive className="size-4 text-muted-foreground" />
                      </div>
                      <div className="flex-1 min-w-0">
                        <p className="text-xs font-medium text-foreground truncate">
                          {row.name}<span className="text-muted-foreground/60">{row.ext}</span>
                        </p>
                        <p className="text-[10px] text-muted-foreground mt-0.5">
                          {row.size} · {row.videos} videos
                        </p>
                      </div>
                      <span
                        className={cn(
                          "size-1.5 rounded-full shrink-0",
                          row.dot === "emerald" ? "bg-emerald-500" : "bg-muted-foreground/40"
                        )}
                        aria-hidden
                      />
                    </m.div>
                  ))}
                </div>
                <p className="text-[10px] text-muted-foreground/70 mt-3 leading-relaxed">
                  Each row = one indexed archive. Click to expand and pick a file.
                </p>
              </div>
            </div>
          </m.div>
        ) : (
          <div className="relative">
            <button
              type="button"
              onClick={() => setIsSourcesDropdownOpen(!isSourcesDropdownOpen)}
              className="flex items-center gap-3 w-full rounded-xl bg-card border border-border p-4 hover:border-white/20 transition-all duration-200 text-left cursor-pointer"
            >
              <div className="size-10 rounded-xl bg-muted flex items-center justify-center flex-shrink-0">
                <HardDrive className="size-5 text-muted-foreground" />
              </div>
              <div className="flex-1 min-w-0">
                <p className="text-sm font-semibold text-foreground">
                  {sources.length} Active {sources.length === 1 ? "Source" : "Sources"}
                </p>
                <div className="flex items-center gap-2 text-xs text-muted-foreground">
                  <span className="flex items-center gap-1">
                    <span className="size-2 rounded-full bg-emerald-500" />
                    {sources.filter(s => !s.isExpired).length} healthy
                  </span>
                  {sources.filter(s => s.isExpired).length > 0 && (
                    <>
                      <span className="size-0.5 rounded-full bg-border" />
                      <span className="flex items-center gap-1">
                        <span className="size-2 rounded-full bg-destructive" />
                        {sources.filter(s => s.isExpired).length} expired
                      </span>
                    </>
                  )}
                </div>
              </div>
              <ChevronDown className={cn(
                "size-4 text-muted-foreground transition-transform duration-200 shrink-0",
                isSourcesDropdownOpen && "rotate-180"
              )} />
            </button>

            <AnimatePresence>
              {isSourcesDropdownOpen && (
                <m.div
                  initial={{ opacity: 0, y: -8, scaleY: 0.95 }}
                  animate={{ opacity: 1, y: 0, scaleY: 1 }}
                  exit={{ opacity: 0, y: -8, scaleY: 0.95 }}
                  transition={{ duration: 0.15, ease: "easeOut" }}
                  className="absolute left-0 right-0 z-50 mt-2 rounded-xl bg-card border border-border shadow-xl shadow-black/40 max-h-[420px] overflow-y-auto origin-top"
                  style={{ transformOrigin: "top" }}
                >
                  {sources.map((source) => (
                    <div
                      key={source.id}
                      className="flex items-center gap-3 p-3 border-b border-border/50 last:border-b-0 hover:bg-white/[0.03] transition-colors"
                    >
                      <div className="relative flex-shrink-0">
                        <div className={cn(
                          "size-10 rounded-lg flex items-center justify-center",
                          source.isExpired ? "bg-destructive/10" : "bg-muted"
                        )}>
                          {source.isExpired ? (
                            <AlertCircle className="size-4 text-destructive" />
                          ) : (
                            <HardDrive className="size-4 text-muted-foreground" />
                          )}
                        </div>
                        <div className={cn(
                          "absolute -top-0.5 -right-0.5 size-2.5 rounded-full border-2 border-card",
                          source.isExpired ? "bg-destructive" : "bg-emerald-500"
                        )} />
                      </div>
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-2 mb-0.5">
                          <p className="text-xs font-medium text-foreground truncate max-w-[55%]" title={source.filename}>
                            {(() => { const f = prettifyFilename(source.filename); return <>{f.display}<span className="text-muted-foreground/60">{f.extension}</span></> })()}
                          </p>
                          <span className="shrink-0 px-1.5 py-0.5 rounded text-[9px] font-semibold bg-muted text-muted-foreground border border-border uppercase">
                            {source.archiveFormat}
                          </span>
                          {source.isExpired && (
                            <span className="shrink-0 px-1.5 py-0.5 rounded text-[9px] font-semibold bg-destructive/10 text-destructive border border-destructive/20 uppercase">
                              Expired
                            </span>
                          )}
                        </div>
                        <div className="flex items-center gap-2 text-[10px] text-muted-foreground">
                          <span>{formatFileSize(source.fileSize)}</span>
                          <span className="size-0.5 rounded-full bg-border" />
                          <span>{source.videoCount} videos</span>
                          <span className="size-0.5 rounded-full bg-border" />
                          <span>{timeAgo(source.createdAt)}</span>
                        </div>
                      </div>
                      <div className="flex items-center gap-1 flex-shrink-0">
                        {source.isExpired ? (
                          <Button variant="ghost" size="icon" className="size-8"
                            onClick={(e) => { e.stopPropagation(); setRefreshModal(source.id); setRefreshUrl(""); setRefreshError("") }}
                            title="Refresh link"
                          >
                            <RefreshCw className="size-3.5" />
                          </Button>
                        ) : (
                          <Button variant="ghost" size="icon" className="size-8"
                            onClick={(e) => { e.stopPropagation(); handleCheckHealth(source.id) }}
                            disabled={!!checkingHealth}
                            title="Check health"
                          >
                            <RefreshCw className={cn("size-3.5", checkingHealth === source.id && "animate-spin")} />
                          </Button>
                        )}
                        <Button variant="ghost" size="icon" className="size-8 hover:text-destructive"
                          onClick={(e) => { e.stopPropagation(); handleDelete(source.id) }}
                          title="Delete source"
                        >
                          <Trash2 className="size-3.5" />
                        </Button>
                      </div>
                    </div>
                  ))}
                </m.div>
              )}
            </AnimatePresence>
          </div>
        )}

        <div className="flex-1 overflow-y-auto min-h-0 mt-4">
          <DdlMediaLibrary
            key={mediaRefreshKey}
            viewMode={viewMode}
            onItemClick={onItemClick ?? (() => {})}
            onFixMatch={onFixMatch ?? (() => {})}
            onDownload={onDownload}
            onDelete={onDelete}
            onWatchTogether={onWatchTogether}
          />
        </div>
      </div>

      {/* Add Archive Dialog */}
      <Dialog open={showAddModal} onOpenChange={(open) => { if (!open && addStep !== "validating" && addStep !== "indexing") setShowAddModal(false) }}>
        <DialogContent className="sm:max-w-lg w-[95vw] sm:w-full overflow-hidden p-0 gap-0">
          {/* Header — compact, identity over chrome */}
          <div className="px-6 pt-6 pb-5 border-b border-border">
            <DialogTitle className="text-base font-semibold leading-none tracking-tight">
              Index a new archive
            </DialogTitle>
            <DialogDescription className="text-xs mt-2 leading-relaxed">
              Paste a hosted link — we probe the headers, index the entries, then stream.
            </DialogDescription>
          </div>

          <div className="px-6 py-5 space-y-4 min-w-0 overflow-hidden">
            {/* URL field with live host/filename preview */}
            <div className="space-y-2 min-w-0">
              <label htmlFor="add-archive-url" className="text-[11px] font-semibold text-muted-foreground uppercase tracking-wider">
                Source URL
              </label>
              <div className="relative group">
                <div className="absolute left-3 top-1/2 -translate-y-1/2 size-7 rounded-md bg-muted border border-border flex items-center justify-center pointer-events-none">
                  <Globe className="size-3.5 text-muted-foreground" />
                </div>
                <Input
                  id="add-archive-url"
                  type="url"
                  placeholder="https://server.com/archive_01.zip"
                  value={addUrl}
                  onChange={e => setAddUrl(e.target.value)}
                  onKeyDown={e => { if (e.key === 'Enter' && addStep === 'idle') handleAdd() }}
                  disabled={addStep === "validating" || addStep === "indexing"}
                  className="w-full h-11 pl-13 pr-3 font-mono text-[13px] tracking-tight"
                  style={{ paddingLeft: "3.25rem" }}
                  autoFocus
                />
              </div>
              {/* Live parse preview — host chip + filename chip, only when the URL is valid */}
              <AnimatePresence>
                {(() => {
                  const { host, filename, ext } = parseArchiveUrl(addUrl)
                  if (!host) return null
                  const isSupported = ext === ".zip" || ext === ".rar"
                  return (
                    <m.div
                      key="preview"
                      initial={{ opacity: 0, height: 0 }}
                      animate={{ opacity: 1, height: "auto" }}
                      exit={{ opacity: 0, height: 0 }}
                      transition={{ duration: 0.18, ease: "easeOut" }}
                      className="overflow-hidden"
                    >
                      <div className="flex items-center gap-1.5 pt-1 min-w-0">
                        <span className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-md bg-muted/60 border border-border text-[11px] text-muted-foreground font-medium min-w-0 max-w-[60%]">
                          <span className="size-1 rounded-full bg-emerald-500 shrink-0" aria-hidden />
                          <span className="truncate">{host}</span>
                        </span>
                        {filename && (
                          <>
                            <ChevronDown className="size-3 text-muted-foreground/60 -rotate-90 shrink-0" />
                            <span className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-md bg-muted/60 border border-border text-[11px] text-foreground font-semibold min-w-0 max-w-[40%]">
                              <span className="truncate">{filename}</span>
                            </span>
                            {ext && !isSupported && (
                              <span className="text-[10px] font-semibold text-amber-400 uppercase tracking-wider shrink-0">
                                ?
                              </span>
                            )}
                          </>
                        )}
                      </div>
                    </m.div>
                  )
                })()}
              </AnimatePresence>
            </div>

            <AnimatePresence mode="wait">
              {addStep === "validating" && (
                <m.div
                  key="validating"
                  initial={{ opacity: 0, y: 10 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, y: -10 }}
                  className="flex items-center gap-3 py-4 text-sm text-muted-foreground"
                >
                  <Loader2 className="size-4 animate-spin" />
                  <span>Validating endpoint…</span>
                </m.div>
              )}

              {addStep === "indexing" && addValidation && (
                <m.div
                  key="indexing"
                  initial={{ opacity: 0, scale: 0.96 }}
                  animate={{ opacity: 1, scale: 1 }}
                  className="space-y-3 w-full min-w-0"
                >
                  {/* File context — anchor for the whole panel. Single row, no duplication. */}
                  <div className="flex items-center gap-3 px-3 py-2.5 rounded-lg bg-muted/50 border border-border min-w-0">
                    <div className="size-7 rounded-md bg-amber-500/10 flex items-center justify-center shrink-0">
                      <FileArchive className="size-3.5 text-amber-400" />
                    </div>
                    <div className="flex-1 min-w-0">
                      <p className="text-sm font-medium text-foreground truncate" title={addValidation.filename}>
                        {(() => { const f = prettifyFilename(addValidation.filename); return <>{f.display}<span className="text-muted-foreground/60">{f.extension}</span></> })()}
                      </p>
                    </div>
                    <span className="text-[11px] font-medium text-muted-foreground tabular-nums shrink-0">
                      {formatFileSize(addValidation.fileSize)}
                    </span>
                  </div>

                  {/* Progress panel */}
                  <div className="rounded-lg bg-muted/30 border border-border p-4 space-y-3 min-w-0 overflow-hidden">
                    {/* Stage + live message */}
                    <div className="flex items-start gap-2.5 min-w-0">
                      <div className="size-1.5 rounded-full bg-foreground mt-1.5 shrink-0 animate-pulse" aria-hidden />
                      <div className="flex-1 min-w-0 space-y-1">
                        <p className="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider">
                          {progressLabel}
                        </p>
                        <AnimatePresence mode="wait">
                          <m.p
                            key={progressMessage}
                            initial={{ opacity: 0, y: 4 }}
                            animate={{ opacity: 1, y: 0 }}
                            exit={{ opacity: 0, y: -4 }}
                            transition={{ duration: 0.18, ease: "easeOut" }}
                            className="text-sm font-medium text-foreground leading-snug"
                          >
                            {progressMessage}
                          </m.p>
                        </AnimatePresence>
                        {progressContext && (
                          <p className="text-xs text-muted-foreground">
                            {progressContext}
                          </p>
                        )}
                      </div>
                    </div>

                    {/* Bar + inline percent/ratio — single row, no stacked hero metric */}
                    <div className="flex items-center gap-3">
                      <div className="h-1 flex-1 bg-muted rounded-full overflow-hidden">
                        <m.div
                          className="h-full rounded-full bg-foreground"
                          initial={{ width: "0%" }}
                          animate={isIndeterminateProgress
                            ? { width: ["18%", "56%", "28%"], x: ["0%", "52%", "0%"] }
                            : { width: `${progressPercent}%` }}
                          transition={isIndeterminateProgress
                            ? { duration: 1.35, repeat: Infinity, ease: "easeInOut" }
                            : { type: "spring", stiffness: 100, damping: 20 }}
                        />
                      </div>
                      <span className="text-[11px] font-semibold text-foreground tabular-nums shrink-0 min-w-[64px] text-right">
                        {isIndeterminateProgress ? (
                          <span className="text-muted-foreground">Working…</span>
                        ) : progressTotal > 0 ? (
                          <>{Math.round(progressPercent)}% · {progressCurrent}/{progressTotal}</>
                        ) : (
                          <span className="text-muted-foreground">—</span>
                        )}
                      </span>
                    </div>

                    {/* Episode discovery — only when relevant */}
                    <AnimatePresence>
                      {addProgress?.episodeTitle && (
                        <m.div
                          initial={{ opacity: 0, height: 0 }}
                          animate={{ opacity: 1, height: "auto" }}
                          exit={{ opacity: 0, height: 0 }}
                          transition={{ duration: 0.2, ease: "easeOut" }}
                          className="overflow-hidden"
                        >
                          <div className="flex items-center gap-2 px-2.5 py-1.5 rounded-md bg-amber-500/5 border border-amber-500/20 min-w-0">
                            <Sparkles className="size-3 text-amber-400 flex-shrink-0" />
                            <span className="text-[11px] text-foreground/80 truncate">
                              <span className="text-muted-foreground">
                                {addProgress.stage === "fetching-episode-metadata" ? "Metadata: " : "Discovered: "}
                              </span>
                              <span className="font-medium">{addProgress.episodeTitle}</span>
                            </span>
                          </div>
                        </m.div>
                      )}
                    </AnimatePresence>
                  </div>
                </m.div>
              )}

              {addStep === "done" && (
                <m.div
                  key="done"
                  initial={{ scale: 0.92, opacity: 0 }}
                  animate={{ scale: 1, opacity: 1 }}
                  className="flex flex-col items-center justify-center py-10 gap-3"
                >
                  <div className="size-12 rounded-full bg-foreground flex items-center justify-center">
                    <CheckCircle className="size-6 text-background" />
                  </div>
                  <p className="text-sm font-semibold text-foreground">Mapping complete</p>
                </m.div>
              )}

              {addStep === "error" && (() => {
                const parsed = parseIndexError(addError)
                return (
                  <m.div
                    key="error"
                    initial={{ opacity: 0, y: 8 }}
                    animate={{ opacity: 1, y: 0 }}
                    transition={{ duration: 0.25, ease: [0.16, 1, 0.3, 1] }}
                    className="rounded-xl border border-destructive/30 bg-destructive/5 overflow-hidden"
                  >
                    <div className="flex items-start gap-3 p-4">
                      <div className="size-8 rounded-lg bg-destructive/15 flex items-center justify-center shrink-0">
                        <AlertCircle className="size-4 text-destructive" />
                      </div>
                      <div className="flex-1 min-w-0 space-y-1.5">
                        <div className="flex items-center gap-2 flex-wrap">
                          <p className="text-sm font-semibold text-foreground">Index failed</p>
                          {parsed.status && (
                            <span className="inline-flex items-center gap-1.5 px-1.5 py-0.5 rounded-md bg-destructive/15 border border-destructive/30 text-[10px] font-bold text-destructive tabular-nums">
                              {parsed.status}
                              {parsed.statusText && <span className="font-medium opacity-80">· {parsed.statusText}</span>}
                            </span>
                          )}
                        </div>
                        {parsed.uri ? (
                          <p className="text-[11px] text-muted-foreground" title={parsed.uri}>
                            Couldn't reach <span className="text-foreground font-medium font-mono">{shortenUriForDisplay(parsed.uri, 56)}</span>
                          </p>
                        ) : (
                          <p className="text-[11px] text-muted-foreground line-clamp-2">{parsed.raw}</p>
                        )}
                      </div>
                    </div>
                    <div className="px-4 py-2.5 border-t border-destructive/15 bg-destructive/[0.03] flex items-start gap-2">
                      <span className="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider shrink-0 mt-px">Fix</span>
                      <p className="text-[11px] text-foreground/80 leading-relaxed">
                        {errorRecoveryHint(parsed)}
                      </p>
                    </div>
                  </m.div>
                )
              })()}
            </AnimatePresence>
          </div>

          {/* Footer — primary action dominates; hint is a quiet footnote above the buttons */}
          <div className="px-6 py-4 border-t border-border bg-card/40">
            {addStep === "idle" && (
              <div className="flex items-center gap-1.5 text-[11px] text-muted-foreground mb-3">
                <HardDrive className="size-3 shrink-0" />
                <span>
                  Host must support <span className="text-foreground font-medium">HTTP Range</span> requests.
                </span>
              </div>
            )}
            <div className="flex items-center justify-between gap-3">
              <div className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
                {addStep === "idle" && (
                  <>
                    <kbd className="inline-flex items-center gap-0.5 px-1.5 h-5 rounded border border-border bg-muted/60 text-[10px] font-mono">
                      <CornerDownLeft className="size-2.5" />
                    </kbd>
                    <span>to start</span>
                  </>
                )}
              </div>
              <div className="flex items-center gap-2">
                <Button
                  variant="ghost"
                  onClick={() => setShowAddModal(false)}
                  disabled={addStep === "validating" || addStep === "indexing"}
                  size="sm"
                >
                  Cancel
                </Button>
                <Button
                  onClick={handleAdd}
                  disabled={!addUrl.trim() || addStep === "validating" || addStep === "indexing" || addStep === "done"}
                  className="min-w-[140px]"
                  size="sm"
                >
                  {addStep === "validating" || addStep === "indexing" ? (
                    <><Loader2 className="size-3.5 mr-2 animate-spin" />{addStep === "validating" ? "Validating…" : "Indexing…"}</>
                  ) : addStep === "error" ? "Retry" : "Start indexing"}
                </Button>
              </div>
            </div>
          </div>
        </DialogContent>
      </Dialog>

      {/* Refresh Link Dialog */}
      <Dialog open={!!refreshModal} onOpenChange={(open) => { if (!open && !refreshing) setRefreshModal(null) }}>
        <DialogContent className="sm:max-w-lg">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <RefreshCw className="size-5" />
              Refresh Session
            </DialogTitle>
            <DialogDescription>Provide a fresh URL for the exact same archive to restore streaming.</DialogDescription>
          </DialogHeader>

          <div className="space-y-4">
            <div className="flex items-start gap-3 p-4 rounded-lg bg-amber-500/5 border border-amber-500/20">
              <AlertCircle className="size-5 text-amber-400 flex-shrink-0 mt-0.5" />
              <p className="text-xs text-muted-foreground leading-relaxed">
                The previous link has expired. Please provide a fresh URL for the <span className="text-foreground font-medium">exact same archive</span>.
              </p>
            </div>

            <div className="space-y-2">
              <label htmlFor="refresh-session-url" className="text-xs font-medium text-muted-foreground">New Session URL</label>
              <Input
                id="refresh-session-url"
                type="url"
                placeholder="https://server.com/new_session_url.zip"
                value={refreshUrl}
                onChange={e => setRefreshUrl(e.target.value)}
                onKeyDown={e => { if (e.key === 'Enter' && refreshModal) handleRefresh(refreshModal) }}
                disabled={refreshing}
                autoFocus
              />
            </div>

            {refreshError && (
              <m.div
                initial={{ height: 0, opacity: 0 }}
                animate={{ height: "auto", opacity: 1 }}
                className="p-3 bg-destructive/10 border border-destructive/20 rounded-lg text-xs text-destructive"
              >
                {refreshError}
              </m.div>
            )}
          </div>

          <DialogFooter className="gap-2">
            <Button variant="ghost" onClick={() => setRefreshModal(null)} disabled={refreshing}>
              Cancel
            </Button>
            <Button
              onClick={() => refreshModal && handleRefresh(refreshModal)}
              disabled={!refreshUrl.trim() || refreshing}
              className="min-w-[160px]"
            >
              {refreshing && <Loader2 className="size-4 mr-2 animate-spin" />}
              Verify & Restore
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

    </div>
    </LazyMotion>
  )
}
