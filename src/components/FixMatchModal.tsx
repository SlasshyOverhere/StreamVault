import { useCallback, useEffect, useMemo, useReducer, useRef } from "react"
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter, DialogDescription } from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Button } from "@/components/ui/button"
import { Check, Film, Loader2, Search, Star, Tv, Globe } from "lucide-react"
import { fixMatch, HybridSearchResult, MediaItem, searchContent } from "@/services/api"
import { useToast } from "@/components/ui/use-toast"
import { cn } from "@/lib/utils"

interface FixMatchModalProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  item: MediaItem | null
  onSuccess: () => void | Promise<void>
}

const TMDB_IMAGE_BASE = "https://image.tmdb.org/t/p/w92"
const DIRECT_MATCH_INPUT_RE =
  /(^\d+$)|(^tt\d+$)|(imdb\.com\/title\/tt\d+)|(themoviedb\.org\/(?:movie|tv)\/\d+)/i

const getResultTitle = (result: HybridSearchResult) => result.title || "Untitled"

const getResultYear = (result: HybridSearchResult): string | null => result.year || null

const extractImdbId = (input: string): string | null => {
  const match = input.match(/tt\d+/)
  return match ? match[0] : null
}

const extractTmdbId = (input: string): string | null => {
  // Direct numeric TMDB ID
  if (/^\d+$/.test(input.trim())) return input.trim()
  // TMDB URL
  const match = input.match(/themoviedb\.org\/(?:movie|tv)\/(\d+)/i)
  return match ? match[1] : null
}

interface State {
  query: string
  results: HybridSearchResult[]
  selectedImdb: string | null
  selectedTmdb: number | null
  isSearching: boolean
  isUpdating: boolean
}

type Action =
  | { type: "RESET" }
  | { type: "INIT_SEARCH"; query: string }
  | { type: "CLEAR_RESULTS" }
  | { type: "SET_QUERY"; query: string }
  | { type: "SET_RESULTS"; results: HybridSearchResult[] }
  | { type: "SELECT_RESULT"; imdb: string | null; tmdb: number | null }
  | { type: "SET_SEARCHING"; value: boolean }
  | { type: "SET_UPDATING"; value: boolean }

const initialState: State = {
  query: "",
  results: [],
  selectedImdb: null,
  selectedTmdb: null,
  isSearching: false,
  isUpdating: false,
}

function reducer(state: State, action: Action): State {
  switch (action.type) {
    case "RESET":
      return initialState
    case "INIT_SEARCH":
      return { ...initialState, query: action.query }
    case "CLEAR_RESULTS":
      return { ...state, results: [], selectedImdb: null, selectedTmdb: null }
    case "SET_QUERY":
      return { ...state, query: action.query }
    case "SET_RESULTS": {
      const first = action.results[0]
      return { ...state, results: action.results, selectedImdb: first?.imdb_id || null, selectedTmdb: first?.tmdb_id || null, isSearching: false }
    }
    case "SELECT_RESULT":
      return { ...state, selectedImdb: action.imdb, selectedTmdb: action.tmdb }
    case "SET_SEARCHING":
      return { ...state, isSearching: action.value }
    case "SET_UPDATING":
      return { ...state, isUpdating: action.value }
    default:
      return state
  }
}

export function FixMatchModal({ open, onOpenChange, item, onSuccess }: FixMatchModalProps) {
  const [state, dispatch] = useReducer(reducer, initialState)
  const { query, results, selectedImdb, selectedTmdb, isSearching, isUpdating } = state
  const searchTokenRef = useRef(0)
  const { toast } = useToast()
  const isDirectMatchInput = useMemo(() => DIRECT_MATCH_INPUT_RE.test(query.trim()), [query])

  const runSearch = useCallback(async (searchQuery: string, mediaType: "movie" | "tv") => {
    const trimmedQuery = searchQuery.trim()
    if (!trimmedQuery || DIRECT_MATCH_INPUT_RE.test(trimmedQuery)) {
      dispatch({ type: "CLEAR_RESULTS" })
      return
    }

    const currentToken = ++searchTokenRef.current
    dispatch({ type: "SET_SEARCHING", value: true })

    try {
      const hybridResults = await searchContent(trimmedQuery, undefined, mediaType)
      if (currentToken !== searchTokenRef.current) return

      dispatch({ type: "SET_RESULTS", results: hybridResults })
    } catch (error) {
      if (currentToken !== searchTokenRef.current) return

      console.error("Search failed", error)
      toast({
        title: "Search Failed",
        description:
          "We couldn't search TMDB for this title. You can paste a TMDB or IMDb URL or ID above to fix the match manually.",
        variant: "destructive"
      })
      dispatch({ type: "SET_SEARCHING", value: false })
    }
  }, [toast])

  useEffect(() => {
    if (!open || !item) {
      searchTokenRef.current += 1
      dispatch({ type: "RESET" })
      return
    }

    const initialQuery = item.title.trim()
    const mediaType = item.media_type === "movie" ? "movie" : "tv"

    dispatch({ type: "INIT_SEARCH", query: initialQuery })
    void runSearch(initialQuery, mediaType)
  }, [open, item, runSearch])

  const handleSearch = async () => {
    if (!item) return
    const mediaType = item.media_type === "movie" ? "movie" : "tv"
    await runSearch(query, mediaType)
  }

  const handleSave = async () => {
    if (!item) return

    const type = item.media_type === "movie" ? "movie" : "tv"
    const trimmedQuery = query.trim()

    let tmdbId: string
    let imdbId: string | undefined

    if (selectedTmdb !== null) {
      tmdbId = selectedTmdb.toString()
      imdbId = selectedImdb || undefined
    } else if (isDirectMatchInput) {
      const extractedImdb = extractImdbId(trimmedQuery)
      const extractedTmdb = extractTmdbId(trimmedQuery)

      if (extractedImdb) {
        imdbId = extractedImdb
        tmdbId = extractedTmdb || ""
      } else if (extractedTmdb) {
        tmdbId = extractedTmdb
      } else {
        tmdbId = trimmedQuery
      }
    } else {
      toast({ title: "Error", description: "Please select a match first", variant: "destructive" })
      return
    }

    if (!tmdbId && !imdbId) {
      toast({ title: "Error", description: "Please select a match first", variant: "destructive" })
      return
    }

    dispatch({ type: "SET_UPDATING", value: true })
    try {
      await fixMatch(item.id, tmdbId, type, imdbId)

      toast({ title: "Success", description: "Metadata updated successfully" })
      onOpenChange(false)

      try {
        await Promise.resolve(onSuccess())
      } catch (refreshError) {
        console.warn("[FixMatch] UI refresh after match update:", refreshError)
      }
    } catch (error) {
      console.error("Failed to fix match", error)
      const errorMessage = typeof error === "string"
        ? error
        : (error as { message?: string })?.message || "Failed to update metadata."

      toast({ title: "Error", description: errorMessage, variant: "destructive" })
    } finally {
      dispatch({ type: "SET_UPDATING", value: false })
    }
  }

  const canSave = Boolean(selectedTmdb !== null || isDirectMatchInput)

  const expectedTypeLabel = item?.media_type === "movie" ? "movie" : "show"

  const handleKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key !== "Enter") return
    event.preventDefault()
    void handleSearch()
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[720px] max-h-[80vh] flex flex-col overflow-hidden">
        <DialogHeader>
          <DialogTitle>Fix Match</DialogTitle>
          <DialogDescription>
            Search TMDB for this {expectedTypeLabel}, or paste a TMDB or IMDb URL or ID, and update its metadata automatically.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-4 min-h-0 flex-1">
          <div className="flex gap-2">
            <Input
              value={query}
              onChange={(e) => dispatch({ type: "SET_QUERY", query: e.target.value })}
              onKeyDown={handleKeyDown}
              placeholder="Search by title, or paste TMDB/IMDb URL or ID..."
              disabled={isSearching || isUpdating}
            />
            <Button onClick={handleSearch} disabled={!query.trim() || isSearching || isUpdating}>
              {isSearching ? <Loader2 className="size-4 animate-spin" /> : <Search className="size-4" />}
              Search
            </Button>
          </div>

          <div className="text-xs space-y-1">
            {query.trim() && !isSearching && !isDirectMatchInput && query.trim().length < 3 && (
              <p className="text-amber-400">Enter at least 3 characters for a valid search.</p>
            )}
            <p className="text-muted-foreground">
            {isSearching
              ? "Searching\u2026"
              : isDirectMatchInput
                ? "Direct match input detected. Click Update Match to fetch metadata from that TMDB/IMDb link or ID."
              : `Found ${results.length} ${results.length === 1 ? "result" : "results"} for ${expectedTypeLabel}s`}
            </p>
          </div>

          <div className="space-y-2 max-h-[360px] overflow-y-auto pr-1">
            {!isSearching && results.length === 0 && !isDirectMatchInput && (
              <div className="rounded-md border border-border/60 bg-muted/20 p-4 text-sm text-muted-foreground">
                No results found. Try adjusting the search title.
              </div>
            )}

            {results.map((result) => {
              const isSelected = selectedTmdb === result.tmdb_id
              const title = getResultTitle(result)
              const year = getResultYear(result)
              const posterUrl = result.tmdb_poster_path
                ? `${TMDB_IMAGE_BASE}${result.tmdb_poster_path}`
                : result.poster_url || null

              return (
                <button
                  key={`${result.media_type}-${result.tmdb_id || result.imdb_id}`}
                  type="button"
                  onClick={() => {
                    dispatch({ type: "SELECT_RESULT", imdb: result.imdb_id || null, tmdb: result.tmdb_id })
                  }}
                  className={cn(
                    "w-full rounded-lg border p-3 text-left transition-colors",
                    "hover:bg-muted/50",
                    isSelected ? "border-primary/70 bg-primary/10" : "border-border/60"
                  )}
                  disabled={isUpdating}
                >
                  <div className="flex items-start gap-3">
                    <div className="w-[50px] h-[75px] rounded-md border border-border/60 overflow-hidden bg-muted/30 flex items-center justify-center shrink-0">
                      {posterUrl ? (
                        <img src={posterUrl} alt={title} className="w-full h-full object-cover" />
                      ) : result.media_type === "movie" ? (
                        <Film className="size-5 text-muted-foreground" />
                      ) : (
                        <Tv className="size-5 text-muted-foreground" />
                      )}
                    </div>

                    <div className="min-w-0 flex-1 space-y-1">
                      <div className="flex items-start justify-between gap-2">
                        <div className="min-w-0">
                          <div className="font-medium line-clamp-1">{title}</div>
                          <div className="text-xs text-muted-foreground">
                            {result.media_type === "movie" ? "Movie" : "TV Show"}
                            {year ? ` \u2022 ${year}` : ""}
                          </div>
                        </div>

                        <div className="flex items-center gap-1.5 shrink-0">
                          {typeof result.imdb_rating === "number" && result.imdb_rating > 0 && (
                            <div className="flex items-center gap-1 text-xs text-yellow-400" title="IMDb Rating">
                              <Globe className="size-3" />
                              {result.imdb_rating.toFixed(1)}
                            </div>
                          )}
                          {typeof result.tmdb_vote_average === "number" && result.tmdb_vote_average > 0 && (
                            <div className="flex items-center gap-1 text-xs text-blue-400" title="TMDB Rating">
                              <Star className="size-3 fill-blue-400 text-blue-400" />
                              {result.tmdb_vote_average.toFixed(1)}
                            </div>
                          )}
                        </div>
                      </div>

                      {result.plot && (
                        <p className="text-xs text-muted-foreground line-clamp-2">{result.plot}</p>
                      )}

                      {result.director && (
                        <p className="text-xs text-muted-foreground/70 line-clamp-1">
                          <span className="font-medium">Director:</span> {result.director}
                        </p>
                      )}

                      {result.imdb_id && (
                        <p className="text-[10px] text-muted-foreground/50">
                          IMDb: {result.imdb_id}
                          {result.tmdb_id ? ` \u2022 TMDB: ${result.tmdb_id}` : ""}
                        </p>
                      )}
                    </div>

                    {isSelected && (
                      <Check className="size-4 text-primary shrink-0 mt-1" />
                    )}
                  </div>
                </button>
              )
            })}
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={isUpdating}>
            Cancel
          </Button>
          <Button onClick={handleSave} disabled={isUpdating || !canSave}>
            {isSearching ? "Searching\u2026" : isUpdating ? "Updating\u2026" : "Update Match"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
