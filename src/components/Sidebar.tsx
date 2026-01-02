import { cn } from "@/lib/utils"
import {
  Play, Film, Tv, History, Settings,
  Globe, Home, RotateCw, Sparkles
} from "lucide-react"
import { motion, AnimatePresence } from "framer-motion"
import { useState, useEffect } from "react"

interface SidebarProps {
  className?: string
  currentView: string
  setView: (view: string) => void
  onOpenSettings: () => void
  onScan: () => void
  theme?: 'dark' | 'light'
  toggleTheme?: () => void
  isScanning?: boolean
  scanProgress?: {
    current: number
    total: number
  } | null
}

export function Sidebar({
  className,
  currentView,
  setView,
  onOpenSettings,
  onScan,
  isScanning = false,
}: SidebarProps) {
  const [isCollapsed, setIsCollapsed] = useState(false);
  const [sidebarWidth, setSidebarWidth] = useState(260);

  useEffect(() => {
    const handleResize = () => {
      if (window.innerWidth < 1024) {
        setIsCollapsed(true);
        setSidebarWidth(80);
      } else {
        setIsCollapsed(false);
        setSidebarWidth(260);
      }
    };

    handleResize();
    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  }, []);

  const menuItems = [
    { id: "home", label: "Home", icon: Home, gradient: "from-violet-500 to-purple-600" },
    { id: "movies", label: "Movies", icon: Film, gradient: "from-pink-500 to-rose-500" },
    { id: "tv", label: "TV Shows", icon: Tv, gradient: "from-blue-500 to-cyan-500" },
    { id: "stream", label: "Discover", icon: Globe, gradient: "from-emerald-500 to-teal-500" },
    { id: "history", label: "History", icon: History, gradient: "from-amber-500 to-orange-500" },
  ];

  return (
    <motion.aside
      animate={{ width: sidebarWidth }}
      transition={{ duration: 0.4, ease: [0.22, 1, 0.36, 1] }}
      className={cn(
        "h-screen flex flex-col relative z-50",
        "bg-gradient-to-b from-card/90 via-background/95 to-background",
        "backdrop-blur-2xl border-r border-white/[0.08]",
        className
      )}
    >
      {/* Decorative gradient blob */}
      <div className="absolute top-0 left-0 w-full h-48 pointer-events-none overflow-hidden">
        <div className="absolute -top-24 -left-24 w-48 h-48 rounded-full bg-primary/20 blur-[80px]" />
      </div>

      {/* Logo Section */}
      <motion.div
        className={cn(
          "relative z-10 flex items-center gap-3.5 px-6 py-7",
          isCollapsed && "justify-center px-0"
        )}
      >
        <motion.div
          whileHover={{ scale: 1.08, rotate: 5 }}
          whileTap={{ scale: 0.95 }}
          className="relative group cursor-pointer"
        >
          {/* Glow effect */}
          <div className="absolute inset-0 rounded-xl bg-gradient-to-br from-primary to-accent opacity-50 blur-lg group-hover:opacity-80 transition-opacity duration-300" />

          {/* Logo container */}
          <div className="relative flex items-center justify-center w-10 h-10 rounded-xl bg-gradient-to-br from-primary/90 to-violet-600 shadow-lg border border-white/20">
            <Play className="w-4 h-4 text-white fill-white ml-0.5 drop-shadow-lg" />
          </div>
        </motion.div>

        <AnimatePresence mode="wait">
          {!isCollapsed && (
            <motion.div
              initial={{ opacity: 0, x: -10 }}
              animate={{ opacity: 1, x: 0 }}
              exit={{ opacity: 0, x: -10 }}
              transition={{ duration: 0.2 }}
              className="flex flex-col"
            >
              <div className="flex items-center gap-1.5">
                <h1 className="text-base font-bold tracking-wide text-foreground">
                  Slasshy
                </h1>
                <Sparkles className="w-3.5 h-3.5 text-primary animate-pulse" />
              </div>
              <span className="text-[10px] text-muted-foreground font-medium tracking-wider uppercase">
                Media Center
              </span>
            </motion.div>
          )}
        </AnimatePresence>
      </motion.div>

      {/* Navigation */}
      <nav className="flex-1 px-3 space-y-1.5 overflow-y-auto py-4">
        {!isCollapsed && (
          <div className="px-3 py-2 text-[10px] font-bold text-muted-foreground/60 uppercase tracking-[0.15em]">
            Navigation
          </div>
        )}

        {menuItems.map((item, index) => {
          const isActive = currentView === item.id;
          return (
            <motion.div
              key={item.id}
              className="relative"
              initial={{ opacity: 0, x: -20 }}
              animate={{ opacity: 1, x: 0 }}
              transition={{ delay: index * 0.05, duration: 0.3 }}
            >
              <motion.button
                onClick={() => setView(item.id)}
                whileHover={{ x: 3 }}
                whileTap={{ scale: 0.98 }}
                className={cn(
                  "relative w-full flex items-center gap-3.5 px-3.5 py-3 rounded-xl text-sm font-medium",
                  "transition-all duration-300 ease-out",
                  isCollapsed && "justify-center px-0",
                  isActive
                    ? "bg-gradient-to-r from-primary/15 to-primary/5 text-white shadow-lg border border-primary/20"
                    : "text-muted-foreground hover:text-white hover:bg-white/[0.04]"
                )}
              >
                {/* Active background glow */}
                {isActive && (
                  <motion.div
                    layoutId="activeGlow"
                    className="absolute inset-0 rounded-xl bg-primary/10 blur-xl opacity-50"
                    transition={{ type: "spring", bounce: 0.2, duration: 0.6 }}
                  />
                )}

                {/* Active indicator line */}
                {isActive && (
                  <motion.div
                    layoutId="activeIndicator"
                    className={cn(
                      "absolute left-0 w-1 bg-gradient-to-b from-primary to-accent rounded-r-full shadow-glow",
                      isCollapsed ? "h-8 top-1/2 -translate-y-1/2" : "h-6 top-1/2 -translate-y-1/2"
                    )}
                    transition={{ type: "spring", bounce: 0.2, duration: 0.6 }}
                  />
                )}

                {/* Icon with gradient background on active */}
                <div className={cn(
                  "relative flex items-center justify-center",
                  isActive && "p-2 rounded-lg bg-gradient-to-br",
                  isActive && item.gradient
                )}>
                  <item.icon className={cn(
                    "w-[18px] h-[18px] transition-all duration-300",
                    isActive ? "text-white drop-shadow-lg" : "text-muted-foreground group-hover:text-white"
                  )} />
                </div>

                {!isCollapsed && (
                  <span className="relative z-10 truncate font-medium">
                    {item.label}
                  </span>
                )}
              </motion.button>

              {/* Tooltip for collapsed mode */}
              {isCollapsed && (
                <div className="absolute left-full ml-2 top-1/2 -translate-y-1/2 px-3 py-1.5 rounded-lg bg-card/95 backdrop-blur-xl border border-white/10 text-xs font-medium text-white opacity-0 group-hover:opacity-100 pointer-events-none shadow-xl transition-opacity whitespace-nowrap">
                  {item.label}
                </div>
              )}
            </motion.div>
          );
        })}
      </nav>

      {/* Bottom Section */}
      <div className={cn(
        "relative z-10 px-3 py-5 space-y-2",
        "border-t border-white/[0.06]",
        "bg-gradient-to-t from-card/50 to-transparent"
      )}>
        {/* Scan Button */}
        <motion.button
          onClick={onScan}
          disabled={isScanning}
          whileHover={{ scale: 1.02 }}
          whileTap={{ scale: 0.98 }}
          className={cn(
            "relative w-full flex items-center gap-3 px-4 py-3 rounded-xl text-sm font-semibold overflow-hidden",
            "transition-all duration-300",
            isCollapsed && "justify-center px-0",
            isScanning
              ? "bg-primary/20 text-primary border border-primary/30"
              : "bg-gradient-to-r from-white/[0.06] to-white/[0.03] hover:from-white/10 hover:to-white/[0.05] border border-white/[0.08] text-muted-foreground hover:text-white"
          )}
        >
          {/* Shimmer effect when scanning */}
          {isScanning && (
            <div className="absolute inset-0 -translate-x-full animate-shimmer bg-gradient-to-r from-transparent via-primary/20 to-transparent" />
          )}

          <RotateCw className={cn(
            "w-4 h-4 flex-shrink-0",
            isScanning && "animate-spin text-primary"
          )} />

          {!isCollapsed && (
            <span className="relative z-10">
              {isScanning ? "Scanning..." : "Update Library"}
            </span>
          )}
        </motion.button>

        {/* Settings Button */}
        <motion.button
          onClick={onOpenSettings}
          whileHover={{ scale: 1.02 }}
          whileTap={{ scale: 0.98 }}
          className={cn(
            "w-full flex items-center gap-3 px-4 py-3 rounded-xl text-sm font-medium",
            "text-muted-foreground hover:text-white",
            "hover:bg-white/[0.04] transition-all duration-200",
            isCollapsed && "justify-center px-0"
          )}
        >
          <Settings className="w-4 h-4 transition-transform duration-300 hover:rotate-90" />
          {!isCollapsed && <span>Settings</span>}
        </motion.button>
      </div>

      {/* Bottom decorative gradient */}
      <div className="absolute bottom-0 left-0 right-0 h-32 pointer-events-none bg-gradient-to-t from-primary/5 to-transparent" />
    </motion.aside>
  );
}
