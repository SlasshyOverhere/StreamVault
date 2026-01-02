import { useState, useEffect } from "react"
import { Play, MoreHorizontal, Edit, Trash2, X, Clock, Check } from "lucide-react"
import { cn } from "@/lib/utils"
import { getCachedImageUrl, MediaItem } from "@/services/api"
import { motion, AnimatePresence } from "framer-motion"
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
  ContextMenuSeparator
} from "@/components/ui/context-menu"

interface MovieCardProps {
  item: MediaItem
  onClick: (item: MediaItem) => void
  onFixMatch: (item: MediaItem) => void
  onRemoveFromHistory?: (item: MediaItem) => void
  onDelete?: (item: MediaItem) => void
  aspectRatio?: "portrait" | "square"
  className?: string
  index?: number
}

export function MovieCard({
  item,
  onClick,
  onFixMatch,
  onRemoveFromHistory,
  onDelete,
  aspectRatio = "portrait",
  className,
  index = 0,
}: MovieCardProps) {
  const [posterUrl, setPosterUrl] = useState<string | null>(null)
  const [isHovered, setIsHovered] = useState(false)
  const [imageLoaded, setImageLoaded] = useState(false)

  const progress = item.progress_percent || (item.resume_position_seconds && item.duration_seconds ? (item.resume_position_seconds / item.duration_seconds) * 100 : 0)
  const isFinished = progress >= 95
  const hasProgress = progress > 0 && !isFinished

  useEffect(() => {
    const loadPoster = async () => {
      if (item.poster_path) {
        const filename = item.poster_path.replace('image_cache/', '')
        const url = await getCachedImageUrl(filename)
        if (url) {
          setPosterUrl(url)
        }
      }
    }
    loadPoster()
  }, [item.poster_path])

  const imageSrc = posterUrl || `https://placehold.co/400x600/0a0a0f/1a1a2e?text=${encodeURIComponent(item.title.slice(0, 2))}`
  const displayInfo = item.year || (item.season_number && item.episode_number ? `S${String(item.season_number).padStart(2, '0')}E${String(item.episode_number).padStart(2, '0')}` : null)

  return (
    <ContextMenu>
      <ContextMenuTrigger>
        <motion.div
          className={cn("group relative", className)}
          onClick={() => onClick(item)}
          onMouseEnter={() => setIsHovered(true)}
          onMouseLeave={() => setIsHovered(false)}
          initial={{ opacity: 0, y: 30, scale: 0.95 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          transition={{
            duration: 0.5,
            delay: index * 0.04,
            ease: [0.22, 1, 0.36, 1]
          }}
        >
          {/* Glow Effect Behind Card */}
          <motion.div
            className="absolute -inset-2 rounded-3xl opacity-0 blur-2xl transition-opacity duration-500 pointer-events-none"
            style={{
              background: `radial-gradient(circle at center, hsl(263 70% 58% / 0.3) 0%, transparent 70%)`,
            }}
            animate={{ opacity: isHovered ? 1 : 0 }}
          />

          {/* Card Container */}
          <motion.div
            className={cn(
              "relative overflow-hidden rounded-2xl cursor-pointer",
              "bg-card/80 backdrop-blur-sm",
              "border border-white/[0.08]",
              "transition-all duration-500 ease-out",
              isHovered && "border-primary/40"
            )}
            animate={{
              y: isHovered ? -10 : 0,
              scale: isHovered ? 1.02 : 1,
            }}
            transition={{ duration: 0.4, ease: [0.22, 1, 0.36, 1] }}
            style={{
              boxShadow: isHovered
                ? '0 25px 50px -12px rgba(0,0,0,0.6), 0 0 40px -10px hsl(263 70% 58% / 0.3)'
                : '0 4px 6px -1px rgba(0,0,0,0.2)',
            }}
          >
            {/* Poster Container */}
            <div className={cn(
              "relative overflow-hidden",
              aspectRatio === "portrait" ? "aspect-[2/3]" : "aspect-square"
            )}>
              {/* Skeleton while loading */}
              <AnimatePresence>
                {!imageLoaded && (
                  <motion.div
                    className="absolute inset-0 skeleton-shimmer"
                    exit={{ opacity: 0 }}
                    transition={{ duration: 0.3 }}
                  />
                )}
              </AnimatePresence>

              {/* Poster Image */}
              <motion.img
                src={imageSrc}
                alt={item.title}
                loading="lazy"
                onLoad={() => setImageLoaded(true)}
                className={cn(
                  "w-full h-full object-cover",
                  "transition-all duration-700 ease-out will-change-transform",
                  imageLoaded ? "opacity-100" : "opacity-0"
                )}
                animate={{
                  scale: isHovered ? 1.12 : 1,
                  filter: isHovered ? 'brightness(1.1) saturate(1.15)' : 'brightness(1) saturate(1)',
                }}
                transition={{ duration: 0.7, ease: [0.22, 1, 0.36, 1] }}
              />

              {/* Gradient Overlay */}
              <motion.div
                className="absolute inset-0 pointer-events-none"
                style={{
                  background: 'linear-gradient(to top, hsl(240 6% 4%) 0%, hsl(240 6% 4% / 0.85) 15%, hsl(240 6% 4% / 0.3) 50%, transparent 100%)',
                }}
                animate={{ opacity: isHovered ? 1 : 0.7 }}
                transition={{ duration: 0.4 }}
              />

              {/* Top Badges */}
              <div className="absolute top-3 left-3 right-3 flex items-start justify-between z-20">
                {/* Progress or Finished Badge */}
                <AnimatePresence>
                  {hasProgress && (
                    <motion.div
                      initial={{ opacity: 0, scale: 0.8, x: -10 }}
                      animate={{ opacity: 1, scale: 1, x: 0 }}
                      exit={{ opacity: 0, scale: 0.8 }}
                      className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg bg-black/60 backdrop-blur-xl border border-white/10 text-xs font-bold text-white shadow-xl"
                    >
                      <Clock className="w-3 h-3 text-primary" />
                      <span>{Math.round(progress)}%</span>
                    </motion.div>
                  )}
                  {isFinished && (
                    <motion.div
                      initial={{ opacity: 0, scale: 0.8, x: -10 }}
                      animate={{ opacity: 1, scale: 1, x: 0 }}
                      exit={{ opacity: 0, scale: 0.8 }}
                      className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg bg-emerald-500/20 backdrop-blur-xl border border-emerald-500/30 text-emerald-400 text-xs font-bold shadow-xl"
                    >
                      <Check className="w-3 h-3" />
                      <span>Watched</span>
                    </motion.div>
                  )}
                </AnimatePresence>

                {/* Options button on hover */}
                <motion.button
                  initial={{ opacity: 0, scale: 0.5 }}
                  animate={{
                    opacity: isHovered ? 1 : 0,
                    scale: isHovered ? 1 : 0.5
                  }}
                  transition={{ duration: 0.2 }}
                  onClick={(e) => { e.stopPropagation(); onFixMatch(item) }}
                  className="ml-auto p-2 rounded-xl bg-black/50 backdrop-blur-xl border border-white/10 text-white/80 hover:text-white hover:bg-black/70 hover:border-white/20 transition-all shadow-xl"
                >
                  <MoreHorizontal className="w-4 h-4" />
                </motion.button>
              </div>

              {/* Play Button - Center */}
              <motion.div
                className="absolute inset-0 flex items-center justify-center z-20"
                initial={{ opacity: 0 }}
                animate={{ opacity: isHovered ? 1 : 0 }}
                transition={{ duration: 0.3 }}
              >
                <motion.div
                  className="relative"
                  initial={{ scale: 0.5 }}
                  animate={{ scale: isHovered ? 1 : 0.5 }}
                  transition={{ duration: 0.3, ease: [0.22, 1, 0.36, 1] }}
                >
                  {/* Glow ring */}
                  <div className="absolute inset-0 rounded-full bg-primary blur-xl opacity-60 scale-150 animate-pulse" />

                  {/* Play button */}
                  <motion.div
                    className="relative w-16 h-16 rounded-full bg-white flex items-center justify-center shadow-2xl cursor-pointer"
                    whileHover={{ scale: 1.1 }}
                    whileTap={{ scale: 0.95 }}
                  >
                    <Play className="w-7 h-7 text-black fill-black ml-1" />
                  </motion.div>
                </motion.div>
              </motion.div>

              {/* Progress Bar */}
              <AnimatePresence>
                {hasProgress && (
                  <motion.div
                    className="absolute bottom-0 left-0 right-0 h-1 bg-white/10 backdrop-blur-sm z-20"
                    initial={{ opacity: 0 }}
                    animate={{ opacity: 1 }}
                    exit={{ opacity: 0 }}
                  >
                    <motion.div
                      className="h-full bg-gradient-to-r from-primary via-violet-400 to-accent relative"
                      initial={{ width: 0 }}
                      animate={{ width: `${progress}%` }}
                      transition={{ duration: 0.8, delay: 0.2, ease: "easeOut" }}
                    >
                      {/* Shimmer on progress bar */}
                      <div className="absolute inset-0 overflow-hidden">
                        <div className="absolute inset-0 -translate-x-full animate-shimmer bg-gradient-to-r from-transparent via-white/30 to-transparent" />
                      </div>
                    </motion.div>
                  </motion.div>
                )}
              </AnimatePresence>
            </div>
          </motion.div>

          {/* Info Below Card */}
          <motion.div
            className="mt-4 space-y-1.5 px-1"
            animate={{ y: isHovered ? 2 : 0 }}
            transition={{ duration: 0.3 }}
          >
            <h3 className={cn(
              "font-semibold text-sm leading-tight line-clamp-1 tracking-tight",
              "transition-colors duration-300",
              isHovered ? "text-white" : "text-white/80"
            )}>
              {item.title}
            </h3>
            <div className="flex items-center gap-2 text-xs font-medium text-muted-foreground/70">
              {displayInfo && (
                <span className="text-muted-foreground">{displayInfo}</span>
              )}
              {item.media_type === 'tvshow' && (
                <>
                  <span className="w-1 h-1 rounded-full bg-primary/50" />
                  <span className="text-primary/70 font-semibold">Series</span>
                </>
              )}
            </div>
          </motion.div>
        </motion.div>
      </ContextMenuTrigger>

      {/* Context Menu */}
      <ContextMenuContent className="min-w-[200px] bg-card/95 backdrop-blur-2xl border-white/10 rounded-xl p-2 shadow-2xl animate-in fade-in zoom-in-95 duration-200">
        <ContextMenuItem
          onClick={() => onClick(item)}
          className="flex items-center gap-3 px-3 py-2.5 rounded-lg cursor-pointer text-sm font-medium focus:bg-white/10 focus:text-white transition-colors"
        >
          <div className="w-8 h-8 rounded-lg bg-primary/20 flex items-center justify-center">
            <Play className="w-4 h-4 text-primary" />
          </div>
          <span>Play Now</span>
        </ContextMenuItem>

        <ContextMenuSeparator className="bg-white/[0.08] my-2" />

        <ContextMenuItem
          onClick={() => onFixMatch(item)}
          className="flex items-center gap-3 px-3 py-2.5 rounded-lg cursor-pointer text-sm font-medium focus:bg-white/10 focus:text-white transition-colors"
        >
          <div className="w-8 h-8 rounded-lg bg-muted/50 flex items-center justify-center">
            <Edit className="w-4 h-4 text-muted-foreground" />
          </div>
          <span>Fix Match</span>
        </ContextMenuItem>

        {onRemoveFromHistory && (
          <>
            <ContextMenuSeparator className="bg-white/[0.08] my-2" />
            <ContextMenuItem
              onClick={() => onRemoveFromHistory(item)}
              className="flex items-center gap-3 px-3 py-2.5 rounded-lg cursor-pointer text-sm font-medium focus:bg-white/10 focus:text-white transition-colors"
            >
              <div className="w-8 h-8 rounded-lg bg-muted/50 flex items-center justify-center">
                <X className="w-4 h-4 text-muted-foreground" />
              </div>
              <span>Remove from History</span>
            </ContextMenuItem>
          </>
        )}

        {onDelete && (
          <>
            <ContextMenuSeparator className="bg-white/[0.08] my-2" />
            <ContextMenuItem
              onClick={() => onDelete(item)}
              className="flex items-center gap-3 px-3 py-2.5 rounded-lg cursor-pointer text-sm font-medium focus:bg-red-500/10 focus:text-red-400 text-red-400/80 transition-colors"
            >
              <div className="w-8 h-8 rounded-lg bg-red-500/15 flex items-center justify-center">
                <Trash2 className="w-4 h-4 text-red-400" />
              </div>
              <span>Delete from Drive</span>
            </ContextMenuItem>
          </>
        )}
      </ContextMenuContent>
    </ContextMenu>
  )
}

// Horizontal Continue Watching Card
interface ContinueCardProps {
  item: MediaItem
  onClick: (item: MediaItem) => void
  index?: number
}

export function ContinueCard({ item, onClick, index = 0 }: ContinueCardProps) {
  const [posterUrl, setPosterUrl] = useState<string | null>(null)
  const [isHovered, setIsHovered] = useState(false)

  const progress = item.progress_percent || (item.resume_position_seconds && item.duration_seconds ? (item.resume_position_seconds / item.duration_seconds) * 100 : 0)

  useEffect(() => {
    const loadPoster = async () => {
      if (item.poster_path) {
        const filename = item.poster_path.replace('image_cache/', '')
        const url = await getCachedImageUrl(filename)
        if (url) {
          setPosterUrl(url)
        }
      }
    }
    loadPoster()
  }, [item.poster_path])

  const imageSrc = posterUrl || `https://placehold.co/200x300/0a0a0f/1a1a2e?text=${encodeURIComponent(item.title.slice(0, 2))}`

  // Calculate remaining time
  const remainingSeconds = item.duration_seconds && item.resume_position_seconds
    ? item.duration_seconds - item.resume_position_seconds
    : null
  const remainingMinutes = remainingSeconds ? Math.ceil(remainingSeconds / 60) : null

  return (
    <motion.div
      initial={{ opacity: 0, x: -30, scale: 0.95 }}
      animate={{ opacity: 1, x: 0, scale: 1 }}
      transition={{ duration: 0.5, delay: index * 0.06, ease: [0.22, 1, 0.36, 1] }}
      onClick={() => onClick(item)}
      onMouseEnter={() => setIsHovered(true)}
      onMouseLeave={() => setIsHovered(false)}
      className="relative group"
    >
      {/* Glow effect */}
      <motion.div
        className="absolute -inset-2 rounded-3xl opacity-0 blur-2xl transition-opacity duration-500 pointer-events-none"
        style={{
          background: `radial-gradient(circle at center, hsl(263 70% 58% / 0.25) 0%, transparent 70%)`,
        }}
        animate={{ opacity: isHovered ? 1 : 0 }}
      />

      <motion.div
        className={cn(
          "relative flex rounded-2xl overflow-hidden cursor-pointer",
          "h-[155px] min-w-[340px] max-w-[420px]",
          "bg-card/80 backdrop-blur-sm border border-white/[0.08]",
          "transition-all duration-400",
          isHovered && "border-primary/40"
        )}
        animate={{
          y: isHovered ? -5 : 0,
          scale: isHovered ? 1.01 : 1,
        }}
        transition={{ duration: 0.3 }}
        style={{
          boxShadow: isHovered
            ? '0 20px 40px -12px rgba(0,0,0,0.5), 0 0 30px -5px hsl(263 70% 58% / 0.2)'
            : '0 4px 6px -1px rgba(0,0,0,0.2)',
        }}
      >
        {/* Blurred background effect */}
        {posterUrl && (
          <div
            className="absolute inset-0 opacity-15 blur-3xl scale-125 pointer-events-none"
            style={{ backgroundImage: `url(${posterUrl})`, backgroundSize: 'cover', backgroundPosition: 'center' }}
          />
        )}

        {/* Gradient overlay */}
        <div className="absolute inset-0 bg-gradient-to-r from-card via-card/95 to-card/80 z-0" />

        {/* Poster */}
        <div className="relative w-[110px] h-full flex-shrink-0 overflow-hidden z-10">
          <motion.img
            src={imageSrc}
            alt={item.title}
            className="w-full h-full object-cover"
            animate={{
              scale: isHovered ? 1.1 : 1,
            }}
            transition={{ duration: 0.6, ease: [0.22, 1, 0.36, 1] }}
          />

          {/* Poster gradient fade */}
          <div className="absolute inset-0 bg-gradient-to-r from-transparent via-transparent to-card pointer-events-none" />

          {/* Play overlay */}
          <motion.div
            className="absolute inset-0 flex items-center justify-center bg-black/40 backdrop-blur-[2px]"
            initial={{ opacity: 0 }}
            animate={{ opacity: isHovered ? 1 : 0 }}
            transition={{ duration: 0.2 }}
          >
            <motion.div
              className="w-12 h-12 rounded-full bg-white flex items-center justify-center shadow-xl"
              initial={{ scale: 0.5 }}
              animate={{ scale: isHovered ? 1 : 0.5 }}
              transition={{ duration: 0.2 }}
            >
              <Play className="w-5 h-5 text-black fill-black ml-0.5" />
            </motion.div>
          </motion.div>
        </div>

        {/* Content */}
        <div className="relative flex-1 p-5 flex flex-col justify-between z-10 min-w-0">
          <div>
            <div className="flex items-center gap-2 mb-2">
              <span className="inline-flex items-center gap-1.5 text-[10px] font-bold uppercase tracking-wider text-primary bg-primary/15 px-2 py-1 rounded-md border border-primary/25">
                <Play className="w-2.5 h-2.5 fill-primary" />
                Resume
              </span>
              {item.media_type === 'tvshow' && (
                <span className="text-[10px] font-semibold text-muted-foreground/60 uppercase tracking-wide">
                  TV Series
                </span>
              )}
            </div>

            <h4 className="font-bold text-[15px] text-white leading-snug line-clamp-1 mb-1 group-hover:text-primary-foreground transition-colors">
              {item.title}
            </h4>
            {item.season_number && item.episode_number && (
              <p className="text-xs text-muted-foreground/70 font-medium">
                Season {item.season_number} · Episode {item.episode_number}
              </p>
            )}
          </div>

          <div className="space-y-2.5">
            {/* Progress bar */}
            <div className="h-1.5 rounded-full bg-white/10 overflow-hidden">
              <motion.div
                className="h-full bg-gradient-to-r from-primary via-violet-400 to-accent relative"
                initial={{ width: 0 }}
                animate={{ width: `${progress}%` }}
                transition={{ duration: 0.8, delay: 0.3 }}
              >
                {/* Shimmer effect */}
                <div className="absolute right-0 top-0 bottom-0 w-6 bg-gradient-to-l from-white/40 to-transparent" />
              </motion.div>
            </div>

            {/* Time info */}
            <div className="flex items-center justify-between text-xs">
              <span className="text-white/80 font-semibold">{Math.round(progress)}% complete</span>
              {remainingMinutes && (
                <span className="text-muted-foreground/70 flex items-center gap-1.5 font-medium">
                  <Clock className="w-3 h-3" />
                  {remainingMinutes}m left
                </span>
              )}
            </div>
          </div>
        </div>
      </motion.div>
    </motion.div>
  )
}
