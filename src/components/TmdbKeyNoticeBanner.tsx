import { KeyRound, Settings, X } from "lucide-react"
import { Button } from "@/components/ui/button"

interface TmdbKeyNoticeBannerProps {
  onOpenSettings: () => void
  onDismiss: () => void
}

/**
 * Persistent in-page banner shown on app startup when no TMDB API key is
 * configured. Stays visible until the user adds a key (parent hides it) or
 * clicks "Don't show again" (parent persists dismissal).
 */
export function TmdbKeyNoticeBanner({ onOpenSettings, onDismiss }: TmdbKeyNoticeBannerProps) {
  return (
    <div
      role="status"
      aria-live="polite"
      className="pointer-events-auto fixed top-14 right-4 z-50 w-[360px] max-w-[calc(100vw-2rem)] rounded-lg border border-border/60 bg-card/95 shadow-2xl backdrop-blur-sm"
    >
      <div className="relative p-4 pr-10">
        <button
          type="button"
          onClick={onDismiss}
          aria-label="Dismiss TMDB key notice"
          className="absolute right-2 top-2 rounded-md p-1 text-muted-foreground transition-colors hover:bg-muted/60 hover:text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
        >
          <X className="size-4" />
        </button>

        <div className="flex items-start gap-3">
          <div className="shrink-0 rounded-md bg-primary/15 p-2 text-primary">
            <KeyRound className="size-5" />
          </div>

          <div className="min-w-0 flex-1 space-y-1.5">
            <h3 className="text-sm font-semibold leading-tight pr-2">
              Set a TMDB API key
            </h3>

            <p className="text-xs leading-relaxed text-muted-foreground">
              Please set a TMDB API key to use the app's full functionality.
            </p>

            <div className="flex flex-wrap items-center gap-2 pt-2">
              <Button size="sm" onClick={onOpenSettings} className="h-8">
                <Settings className="size-3.5" />
                Open Settings
              </Button>
              <Button
                size="sm"
                variant="ghost"
                onClick={onDismiss}
                className="h-8 text-muted-foreground hover:text-foreground"
              >
                Don't show again
              </Button>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}
