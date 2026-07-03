import { cn } from "@/lib/utils"
import {
  Settings,
  Home, RotateCw, Cloud, Clapperboard, Download, Link2, BarChart3, Radio,
  Pin, PinOff, ShieldCheck, Copy, Calendar
} from "lucide-react"
import { LazyMotion, domAnimation, m } from "framer-motion"
import { useState, useEffect, useRef, useCallback, useSyncExternalStore } from "react"
import { isGDriveConnected, getGDriveAccountInfo, DriveAccountInfo, formatStorageSize } from "@/services/gdrive"

interface SidebarProps {
  className?: string
  currentView: string
  setView: (view: string) => void
  onOpenSettings: () => void
  onCloudScan?: () => void
  theme?: 'dark' | 'light'
  toggleTheme?: () => void
  isScanning?: boolean
  isCloudIndexing?: boolean
  scanProgress?: {
    current: number
    total: number
  } | null
  showCloudTab?: boolean
  betaEnabled?: boolean
  downloadJobCount?: number
  onSyncValidator?: () => void
  onDuplicateDetector?: () => void
}

export function Sidebar({
  className,
  currentView,
  setView,
  onOpenSettings,
  onCloudScan,
  isScanning = false,
  isCloudIndexing = false,
  showCloudTab = true,
  betaEnabled: _betaEnabled = false,
  downloadJobCount = 0,
  onSyncValidator,
  onDuplicateDetector,
}: SidebarProps) {
  const subscribeWindowResize = useCallback((callback: () => void) => {
    window.addEventListener("resize", callback);
    return () => window.removeEventListener("resize", callback);
  }, []);
  const getWindowWidth = useCallback(() => window.innerWidth, []);
  const windowWidth = useSyncExternalStore(subscribeWindowResize, getWindowWidth);
  const [isHovered, setIsHovered] = useState(false);
  const [isPinned, setIsPinned] = useState(() => localStorage.getItem('slasshyvault_sidebar_pinned') === 'true');
  const [gdriveConnected, setGdriveConnected] = useState(false);
  const [gdriveInfo, setGdriveInfo] = useState<DriveAccountInfo | null>(null);
  const hoverTimeoutRef = useRef<NodeJS.Timeout | null>(null);

  const handleMouseEnter = () => {
    if (hoverTimeoutRef.current) clearTimeout(hoverTimeoutRef.current);
    setIsHovered(true);
  };

  const handleMouseLeave = () => {
    // Clear any existing timer
    if (hoverTimeoutRef.current) clearTimeout(hoverTimeoutRef.current);
    
    // Start 0.1 second timer AFTER leaving the sidebar
    hoverTimeoutRef.current = setTimeout(() => {
      setIsHovered(false);
    }, 100);
  };

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      if (hoverTimeoutRef.current) clearTimeout(hoverTimeoutRef.current);
    };
  }, []);

  useEffect(() => {
    localStorage.setItem('slasshyvault_sidebar_pinned', String(isPinned));
  }, [isPinned]);

  // Listen for pin toggle from command palette
  useEffect(() => {
    const handler = () => {
      setIsPinned(localStorage.getItem('slasshyvault_sidebar_pinned') === 'true');
    };
    window.addEventListener('sidebar-pin-toggle', handler);
    return () => window.removeEventListener('sidebar-pin-toggle', handler);
  }, []);

  const isCollapsed = !isHovered && !isPinned;
  const sidebarWidth = isCollapsed ? 64 : (windowWidth < 1100 ? 232 : 264);

  // Fetch Google Drive info
  useEffect(() => {
    const fetchGdriveInfo = async () => {
      const connected = await isGDriveConnected();
      setGdriveConnected(connected);
      if (connected) {
        const info = await getGDriveAccountInfo();
        setGdriveInfo(info);
      }
    };
    fetchGdriveInfo();
    const interval = setInterval(fetchGdriveInfo, 60000);
    return () => clearInterval(interval);
  }, []);

  const menuItems = [
    { id: "home", label: "Home", icon: Home },
    { id: "cloud", label: "Library", icon: Cloud, hidden: !showCloudTab },
    { id: "remote", label: "External", icon: Radio },
    { id: "directlinks", label: "Direct Links", icon: Link2 },
    { id: "reminders", label: "Watchlist", icon: Clapperboard },
    { id: "downloads", label: "Downloads", icon: Download, badge: downloadJobCount > 0 ? String(downloadJobCount) : undefined },
    { id: "history", label: "History & Analytics", icon: BarChart3 },
    { id: "calendar", label: "Calendar", icon: Calendar },
  ].filter(item => !item.hidden);

  return (
    <LazyMotion features={domAnimation}>
    <m.aside
      data-tour="sidebar"
      className={cn(
        "h-screen flex flex-col z-[100]",
        "bg-[#0D0D0D]",
        "border-r border-white/[0.05] shadow-2xl",
        "will-change-[width]",
        className
      )}
      animate={{ width: sidebarWidth }}
      transition={{ 
        type: "spring", 
        stiffness: 400, 
        damping: 40,
        mass: 1,
        restDelta: 0.001
      }}
      onMouseEnter={handleMouseEnter}
      onMouseLeave={handleMouseLeave}
      onFocus={handleMouseEnter}
      onBlur={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node)) {
          handleMouseLeave()
        }
      }}
    >
      {/* Glossy Overlay */}
      <div className="absolute inset-0 bg-gradient-to-b from-white/[0.02] to-transparent pointer-events-none" />

      <div className={cn("flex-1 px-3 pt-14 pb-3 flex flex-col", isCollapsed ? "px-1.5 pt-12" : "")}>
        {/* Navigation Items */}
        <div className="flex-1 flex items-start pt-12">
          <nav className="w-full gap-y-2 flex flex-col overflow-visible">
            {menuItems.map((item) => {
              const isActive = currentView === item.id;

              return (
                  <button
                    type="button"
                    key={item.id}
                    data-tour={`nav-${item.id}`}
                    aria-label={item.label}
                    aria-current={isActive ? 'page' : undefined}
                    onClick={() => setView(item.id)}
                  className={cn(
                    "group relative w-full flex items-center gap-3 px-3.5 py-3 rounded-xl transition-colors duration-300 focus-ring",
                    isActive
                      ? "bg-white/[0.12] text-white shadow-[0_0_25px_rgba(255,255,255,0.08)] border border-white/20 backdrop-blur-md"
                      : "text-neutral-500 hover:text-neutral-200 hover:bg-white/[0.04]",

                    isCollapsed ? "justify-center px-0" : ""
                  )}
                >
                  {/* Active Indicator & Glow */}
                  {isActive && (
                    <>
                      <m.div
                        layoutId="active-glow"
                        className="absolute inset-0 rounded-xl bg-gradient-to-r from-white/10 to-transparent blur-xl opacity-50"
                        transition={{ type: "spring", stiffness: 300, damping: 30 }}
                      />

                      <m.div
                        layoutId="active-pill"
                        className="absolute left-1 inset-y-0 my-auto w-1 h-6 bg-white rounded-full shadow-[0_0_15px_rgba(255,255,255,0.6)] z-10"
                        transition={{ type: "spring", stiffness: 300, damping: 30 }}
                      />

                    </>
                  )}

                  <div className="relative">
                    <item.icon className={cn(
                      "size-5 transition-all duration-300",
                      isActive ? "text-white drop-shadow-white" : "text-neutral-500 group-hover:text-neutral-300"
                    )} />
                    {isCollapsed && item.badge && (
                      <div className="absolute -right-1.5 -top-1.5 flex h-4 min-w-[16px] items-center justify-center rounded-full border border-white/50 bg-white px-1 text-[8px] font-black text-black shadow-[0_0_10px_rgba(255,255,255,0.4)]">
                        {item.badge}
                      </div>
                    )}
                  </div>

                  {!isCollapsed && (
                    <>
                      <span className={cn(
                        "text-sm font-semibold tracking-wide transition-colors duration-300",
                        isActive ? "text-white" : "text-neutral-500 group-hover:text-neutral-200"
                      )}>
                        {item.label}
                      </span>
                      {item.badge && (
                        <span className={cn(
                          "ml-auto min-w-6 rounded-full px-2 py-0.5 text-[10px] font-black tracking-[0.08em] text-center border transition-all duration-300 shadow-[0_0_15px_rgba(255,255,255,0.15)]",
                          isActive
                            ? "border-white/60 bg-white text-black"
                            : "border-white/20 bg-white/10 text-neutral-400 group-hover:bg-white/20"
                        )}>
                          {item.badge}
                        </span>
                      )}
                    </>
                  )}

                  {/* Tooltip for collapsed mode */}
                  {isCollapsed && (
                    <div className="absolute left-full ml-4 z-[60] whitespace-nowrap rounded-lg border border-white/10 bg-[#141414] px-3 py-2 shadow-2xl pointer-events-none opacity-0 translate-x-1 transition-all duration-200 [transition-delay:0ms] group-hover:[transition-delay:100ms] group-hover:opacity-100 group-hover:translate-x-0 group-focus:opacity-100 group-focus:translate-x-0">
                      <span className="text-xs font-semibold text-white">Open {item.label}</span>
                      {item.badge && (
                        <span className="text-xs font-bold text-white tracking-wider">{` • ${item.badge}`}</span>
                      )}
                    </div>
                  )}
                </button>
              )
            })}
          </nav>
        </div>

      </div>

      {/* Footer / Status Area */}
      <div className={cn("mt-auto space-y-3 border-t border-white/[0.04] bg-white/[0.01]", isCollapsed ? "p-2.5" : "p-4")}>

        {/* Cloud Sync Status */}
        {gdriveConnected && (
          <div className="gap-y-3 flex flex-col items-center">
            {onCloudScan && (
              <button
                type="button"
                data-tour="scan-library-btn"
                onClick={onCloudScan}
                disabled={isCloudIndexing || isScanning}
                aria-label="Update Library"
                className={cn(
                  "w-full flex items-center justify-between transition-all duration-300 focus-ring",
                  isCollapsed
                    ? "size-10 justify-center rounded-full bg-white/[0.04] border border-white/[0.08] hover:bg-white/[0.08]"
                    : "px-4 py-2.5 rounded-xl bg-white/[0.04] border border-white/[0.06] hover:bg-white/[0.08] hover:border-white/10 group",
                  isCloudIndexing ? "opacity-70 cursor-wait" : ""
                )}
                title={isCollapsed ? "Update Library" : ""}
              >
                <div className={cn("flex items-center gap-3", isCollapsed ? "justify-center" : "")}>
                  <RotateCw className={cn("size-4 text-white", isCloudIndexing && "animate-spin")} />
                  {!isCollapsed && <span className="text-xs font-bold text-neutral-300">Update Library</span>}
                </div>
                {!isCollapsed && <div className="size-1.5 rounded-full bg-white animate-pulse shadow-[0_0_8px_rgba(255,255,255,0.5)]" />}
              </button>
            )}

            {onSyncValidator && (
              <button
                type="button"
                onClick={onSyncValidator}
                disabled={isCloudIndexing || isScanning}
                aria-label="Sync Validator"
                className={cn(
                  "w-full flex items-center justify-between transition-all duration-300 focus-ring",
                  isCollapsed
                    ? "size-10 justify-center rounded-full bg-white/[0.04] border border-white/[0.08] hover:bg-white/[0.08]"
                    : "px-4 py-2.5 rounded-xl bg-white/[0.04] border border-white/[0.06] hover:bg-white/[0.08] hover:border-white/10 group",
                  (isCloudIndexing || isScanning) ? "opacity-50 cursor-not-allowed" : ""
                )}
                title={isCollapsed ? "Sync Validator" : ""}
              >
                <div className={cn("flex items-center gap-3", isCollapsed ? "justify-center" : "")}>
                  <ShieldCheck className="size-4 text-emerald-400" />
                  {!isCollapsed && <span className="text-xs font-bold text-neutral-300">Sync Validator</span>}
                </div>
              </button>
            )}

            {onDuplicateDetector && (
              <button
                type="button"
                onClick={onDuplicateDetector}
                aria-label="Duplicate Detector"
                className={cn(
                  "w-full flex items-center justify-between transition-all duration-300 focus-ring",
                  isCollapsed
                    ? "size-10 justify-center rounded-full bg-white/[0.04] border border-white/[0.08] hover:bg-white/[0.08]"
                    : "px-4 py-2.5 rounded-xl bg-white/[0.04] border border-white/[0.06] hover:bg-white/[0.08] hover:border-white/10 group"
                )}
                title={isCollapsed ? "Duplicate Detector" : ""}
              >
                <div className={cn("flex items-center gap-3", isCollapsed ? "justify-center" : "")}>
                  <Copy className="size-4 text-amber-400" />
                  {!isCollapsed && <span className="text-xs font-bold text-neutral-300">Duplicate Detector</span>}
                </div>
              </button>
            )}

            {/* Storage Card - Premium Polished Version */}
            {gdriveInfo && gdriveInfo.storage_used !== undefined && gdriveInfo.storage_limit !== undefined && (
              <div className={cn(
                "w-full transition-all duration-300 relative overflow-hidden group/storage",
                isCollapsed 
                  ? "size-11 rounded-full bg-white/[0.03] border border-white/[0.08] shadow-[0_0_15px_rgba(255,255,255,0.02)]" 
                  : "h-[88px] rounded-2xl bg-white/[0.03] border border-white/[0.06] px-4 py-3 shadow-[0_8px_30px_rgba(0,0,0,0.2)]"
              )}>
                {/* Glossy Sheen Overlay */}
                <div className="absolute inset-0 bg-gradient-to-br from-white/[0.02] to-transparent pointer-events-none" />
                
                {/* Expanded Content Layer */}
                <div className={cn(
                  "transition-all duration-300 ease-in-out h-full flex flex-col justify-center",
                  isCollapsed ? "opacity-0 translate-x-[-10px] invisible" : "opacity-100 translate-x-0 visible"
                )}>
                  <div className="space-y-2.5">
                    <div className="flex justify-between items-baseline">
                      <div className="flex items-center gap-1.5">
                        <div className="w-1 h-3 bg-white/20 rounded-full" />
                        <span className="text-[10px] font-black text-white/40 uppercase tracking-[0.15em]">Storage</span>
                      </div>
                      <span className="text-[10px] font-black text-white/90 bg-white/10 px-2 py-0.5 rounded-full border border-white/5">
                        {Math.round((gdriveInfo.storage_used / gdriveInfo.storage_limit) * 100)}%
                      </span>
                    </div>
                    
                    <div className="relative h-1.5 w-full bg-white/[0.03] rounded-full overflow-hidden border border-white/[0.02] shadow-inner">
                      <div
                        className="h-full bg-gradient-to-r from-neutral-500 via-white to-neutral-400 rounded-full transition-all duration-700 relative"
                        style={{ 
                          width: `${Math.min((gdriveInfo.storage_used / gdriveInfo.storage_limit) * 100, 100)}%`,
                          boxShadow: '0 0 10px rgba(255,255,255,0.2)' 
                        }}
                      >
                        {/* Shimmer Effect */}
                        <div className="absolute inset-0 bg-gradient-to-r from-transparent via-white/20 to-transparent animate-shimmer" style={{ backgroundSize: '200% 100%' }} />
                      </div>
                    </div>

                    <div className="flex justify-between items-center">
                      <span className="text-[9px] font-black text-white/30 uppercase tracking-wider">
                        {formatStorageSize(gdriveInfo.storage_used)} / {formatStorageSize(gdriveInfo.storage_limit)}
                      </span>
                    </div>
                  </div>
                </div>

                {/* Collapsed Content Layer (Modern Circle Indicator) */}
                <div className={cn(
                  "absolute inset-0 flex items-center justify-center transition-all duration-300",
                  !isCollapsed ? "opacity-0 scale-90 invisible" : "opacity-100 scale-100 visible"
                )}>
                  <div className="relative flex items-center justify-center">
                    {/* Outer Glow Ring */}
                    <div className="absolute inset-[-4px] rounded-full border border-white/[0.03] animate-pulse-soft" />
                    <span className="text-[9px] font-black text-white/50 tracking-tighter group-hover/storage:text-white transition-colors">
                      {Math.round((gdriveInfo.storage_used / gdriveInfo.storage_limit) * 100)}%
                    </span>
                  </div>
                </div>
              </div>
            )}
          </div>
        )}

        {/* Footer Actions */}
        {isCollapsed ? (
          <div className="space-y-1.5">
            <div className="group relative">
              <button
                type="button"
                onClick={() => setIsPinned(!isPinned)}
                title={isPinned ? "Unpin sidebar" : "Pin sidebar"}
                aria-label={isPinned ? "Unpin sidebar" : "Pin sidebar"}
                className={cn(
                  "w-full h-10 rounded-xl border transition-all duration-200 flex items-center justify-center focus-ring",
                  isPinned
                    ? "text-amber-400 bg-amber-500/10 border-amber-500/25"
                    : "text-neutral-500 border-white/[0.06] bg-white/[0.03] hover:bg-white/[0.08] hover:text-white hover:border-white/10"
                )}
              >
                {isPinned ? <PinOff className="size-4" /> : <Pin className="size-4" />}
              </button>
              <div className="absolute left-full top-1/2 ml-3 -translate-y-1/2 z-[60] whitespace-nowrap rounded-lg border border-white/10 bg-[#141414] px-3 py-2 shadow-2xl pointer-events-none opacity-0 translate-x-1 transition-all duration-200 [transition-delay:0ms] group-hover:[transition-delay:100ms] group-hover:opacity-100 group-hover:translate-x-0 group-focus:opacity-100 group-focus:translate-x-0">
                <span className="text-xs font-semibold text-white">{isPinned ? "Unpin sidebar" : "Pin sidebar"}</span>
              </div>
            </div>
            <div className="group relative">
              <button
                type="button"
                data-tour="settings-btn"
                onClick={onOpenSettings}
                title="Open settings"
                aria-label="Open settings"
                className="w-full h-10 rounded-xl border border-white/[0.06] bg-white/[0.03] transition-colors duration-200 flex items-center justify-center text-neutral-500 hover:bg-white/[0.08] hover:text-white hover:border-white/10 focus-ring"
              >
                <Settings className="size-4 transition-transform duration-200 group-hover:rotate-45" />
              </button>
              <div className="absolute left-full top-1/2 ml-3 -translate-y-1/2 z-[60] whitespace-nowrap rounded-lg border border-white/10 bg-[#141414] px-3 py-2 shadow-2xl pointer-events-none opacity-0 translate-x-1 transition-all duration-200 [transition-delay:0ms] group-hover:[transition-delay:100ms] group-hover:opacity-100 group-hover:translate-x-0 group-focus:opacity-100 group-focus:translate-x-0">
                <span className="text-xs font-semibold text-white">Open Settings</span>
              </div>
            </div>

          </div>
        ) : (
          <div className="flex flex-col gap-2">
              <button
                type="button"
                onClick={() => setIsPinned(!isPinned)}
                title={isPinned ? "Unpin sidebar" : "Pin sidebar"}
                aria-label={isPinned ? "Unpin sidebar" : "Pin sidebar"}
                className={cn(
                  "group h-11 w-full rounded-xl border transition-all duration-200 flex items-center justify-center gap-2.5 px-3 focus-ring",
                  isPinned
                    ? "border-amber-500/25 bg-amber-500/10 text-amber-400 shadow-sm shadow-amber-500/5"
                    : "border-white/[0.06] bg-white/[0.03] text-neutral-500 hover:bg-white/[0.08] hover:text-white hover:border-white/10"
                )}
              >
                {isPinned ? <PinOff className="size-4" /> : <Pin className="size-4" />}
                <span className="text-[11px] font-semibold tracking-wider uppercase">
                  {isPinned ? "Pinned" : "Pin sidebar"}
                </span>
              </button>
              <button
                type="button"
                data-tour="settings-btn"
                onClick={onOpenSettings}
                title="Open settings"
                aria-label="Open settings"
                className="group h-11 w-full rounded-xl border border-white/[0.06] bg-white/[0.03] transition-all duration-200 flex items-center justify-center gap-2.5 px-3 text-neutral-500 hover:bg-white/[0.08] hover:text-white hover:border-white/10 focus-ring"
              >
                <Settings className="size-4 transition-transform duration-200 group-hover:rotate-45" />
                <span className="text-[11px] font-semibold tracking-wider uppercase">Settings</span>
              </button>
          </div>
        )}
      </div>
    </m.aside>
    </LazyMotion>
  )
}
