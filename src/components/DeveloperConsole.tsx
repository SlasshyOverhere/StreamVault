import { useState, useEffect, useRef, useCallback } from "react";
import { LazyMotion, m, AnimatePresence, domAnimation } from "framer-motion";
import { invoke } from "@tauri-apps/api/core";
import { Trash2, Terminal, ChevronDown, ChevronUp } from "lucide-react";
import { cn } from "@/lib/utils";

interface LogEntry {
  id: number;
  level: "info" | "warn" | "error" | "debug";
  message: string;
  timestamp: Date;
  source: "frontend" | "backend";
}

type LogFilter = "all" | "info" | "warn" | "error";

const MAX_VISIBLE_LOGS = 500;

const originalConsole = {
  log: console.log,
  warn: console.warn,
  error: console.error,
  debug: console.debug,
  info: console.info,
};

const levelStyles: Record<string, string> = {
  info: "text-blue-300",
  warn: "text-yellow-300",
  error: "text-red-300",
  debug: "text-gray-400",
};

const levelBadge: Record<string, string> = {
  info: "bg-blue-500/20 text-blue-300",
  warn: "bg-yellow-500/20 text-yellow-300",
  error: "bg-red-500/20 text-red-300",
  debug: "bg-gray-500/20 text-gray-400",
};

const sourceBadge: Record<string, string> = {
  frontend: "bg-zinc-600/30 text-zinc-400",
  backend: "bg-emerald-600/30 text-emerald-400",
};

export function DeveloperConsole() {
  const [open, setOpen] = useState(false);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [filter, setFilter] = useState<LogFilter>("all");
  const [autoScroll, setAutoScroll] = useState(true);
  const listRef = useRef<HTMLDivElement>(null);
  const idCounter = useRef(0);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Intercept console methods
  useEffect(() => {
    const handler = (level: LogEntry["level"]) => (...args: unknown[]) => {
      const message = args.map((a) => (typeof a === "object" ? safeStringify(a) : String(a))).join(" ");
      const entry: LogEntry = {
        id: ++idCounter.current,
        level,
        message,
        timestamp: new Date(),
        source: "frontend",
      };
      setLogs((prev) => {
        const next = [...prev, entry];
        if (next.length > MAX_VISIBLE_LOGS * 2) {
          return next.slice(next.length - MAX_VISIBLE_LOGS);
        }
        return next;
      });
    };

    console.log = handler("info");
    console.warn = handler("warn");
    console.error = handler("error");
    console.debug = handler("debug");
    console.info = handler("info");

    return () => {
      console.log = originalConsole.log;
      console.warn = originalConsole.warn;
      console.error = originalConsole.error;
      console.debug = originalConsole.debug;
      console.info = originalConsole.info;
    };
  }, []);

  // Poll for backend logs
  const fetchBackendLogs = useCallback(async () => {
    try {
      const newLogs = await invoke<string[]>("get_recent_logs");
      if (newLogs.length > 0) {
        for (const msg of newLogs) {
          const entry: LogEntry = {
            id: ++idCounter.current,
            level: "info",
            message: msg,
            timestamp: new Date(),
            source: "backend",
          };
          setLogs((prev) => {
            const next = [...prev, entry];
            if (next.length > MAX_VISIBLE_LOGS * 2) {
              return next.slice(next.length - MAX_VISIBLE_LOGS);
            }
            return next;
          });
        }
      }
    } catch {
      // Silently fail
    }
  }, []);

  useEffect(() => {
    if (open) {
      fetchBackendLogs();
      pollRef.current = setInterval(fetchBackendLogs, 1500);
    } else {
      if (pollRef.current) {
        clearInterval(pollRef.current);
        pollRef.current = null;
      }
    }
    return () => {
      if (pollRef.current) {
        clearInterval(pollRef.current);
      }
    };
  }, [open, fetchBackendLogs]);

  // Auto-scroll
  useEffect(() => {
    if (autoScroll && listRef.current) {
      listRef.current.scrollTop = listRef.current.scrollHeight;
    }
  }, [logs, autoScroll]);

  const filteredLogs = logs.filter((entry) => {
    if (filter === "all") return true;
    return entry.level === filter;
  });

  const clearLogs = async () => {
    setLogs([]);
    try {
      await invoke("clear_logs");
    } catch {}
  };

  const feCount = logs.filter((l) => l.source === "frontend").length;
  const beCount = logs.filter((l) => l.source === "backend").length;
  const filteredCount = filteredLogs.length;
  const totalCount = logs.length;

  return (
    <LazyMotion features={domAnimation}>
      <div className="fixed bottom-32 right-4 z-[9999] flex flex-col items-end gap-2">
      {/* Toggle button */}
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className={cn(
          "flex items-center gap-2 px-3 py-2 rounded-xl text-xs font-medium transition-all",
          open
            ? "bg-zinc-800 text-white border border-zinc-600"
            : "bg-zinc-900/80 text-zinc-300 border border-zinc-700 hover:bg-zinc-800",
        )}
        aria-label="Toggle developer console"
      >
        <Terminal className="size-3.5" />
        Console
        {open ? <ChevronDown className="size-3" /> : <ChevronUp className="size-3" />}
      </button>

      <AnimatePresence>
        {open && (
          <m.div
            initial={{ opacity: 0, y: 10, scale: 0.95 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 10, scale: 0.95 }}
            transition={{ duration: 0.15 }}
            className="w-[600px] max-w-[90vw] h-[400px] max-h-[60vh] rounded-xl border border-zinc-700 bg-zinc-900/95 backdrop-blur-sm shadow-2xl flex flex-col overflow-hidden"
          >
            {/* Header */}
            <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-700 flex-shrink-0">
              <div className="flex items-center gap-2">
                <Terminal className="size-4 text-zinc-400" />
                <span className="text-xs font-medium text-zinc-200">Developer Console</span>
                <span className="text-[10px] text-zinc-500">({totalCount} entries)</span>
              </div>
              <div className="flex items-center gap-1.5">
                {/* Filter buttons */}
                {(["all", "info", "warn", "error"] as const).map((f) => (
                  <button
                    type="button"
                    key={f}
                    onClick={() => setFilter(f)}
                    className={cn(
                      "px-1.5 py-0.5 text-[10px] rounded transition-colors",
                      filter === f
                        ? "bg-zinc-600 text-zinc-200"
                        : "text-zinc-500 hover:text-zinc-300",
                    )}
                  >
                    {f === "all" ? "ALL" : f.toUpperCase()}
                  </button>
                ))}
                <div className="w-px h-4 bg-zinc-700 mx-1" />
                <button
                  type="button"
                  onClick={() => setAutoScroll(!autoScroll)}
                  className={cn(
                    "px-1.5 py-0.5 text-[10px] rounded transition-colors",
                    autoScroll ? "bg-zinc-600 text-zinc-200" : "text-zinc-500 hover:text-zinc-300",
                  )}
                >
                  AUTO
                </button>
                <button
                  type="button"
                  onClick={clearLogs}
                  className="p-1 text-zinc-500 hover:text-zinc-300 transition-colors"
                  title="Clear logs"
                >
                  <Trash2 className="size-3.5" />
                </button>
              </div>
            </div>

            {/* Log list */}
            <div ref={listRef} className="flex-1 overflow-y-auto p-2 space-y-0.5 font-mono text-[11px] leading-relaxed">
              {filteredLogs.length === 0 ? (
                <div className="text-zinc-600 text-center mt-8 text-xs">No log entries</div>
              ) : (
                filteredLogs.map((entry) => (
                  <div key={entry.id} className="flex items-start gap-1.5 hover:bg-zinc-800/50 px-1 py-0.5 rounded">
                    <span className={cn("text-[9px] font-semibold uppercase flex-shrink-0 w-10 text-center rounded", levelBadge[entry.level])}>
                      {entry.level === "info" ? "INF" : entry.level === "warn" ? "WRN" : entry.level === "error" ? "ERR" : "DBG"}
                    </span>
                    <span className={cn("text-[9px] flex-shrink-0 rounded px-0.5", sourceBadge[entry.source])}>
                      {entry.source === "backend" ? "RS" : "JS"}
                    </span>
                    <span className="text-zinc-600 flex-shrink-0 w-12">
                      {entry.timestamp.toLocaleTimeString("en-US", { hour12: false })}
                    </span>
                    <span className={cn("break-all min-w-0", levelStyles[entry.level])}>
                      {entry.message}
                    </span>
                  </div>
                ))
              )}
            </div>

            {/* Footer */}
            <div className="flex items-center justify-between px-3 py-1.5 border-t border-zinc-700 flex-shrink-0">
              <span className="text-[10px] text-zinc-600">
                FE: {feCount} · BE: {beCount}
              </span>
              <span className="text-[10px] text-zinc-600">
                {filter !== "all" ? `Showing ${filteredCount}/${totalCount}` : `${totalCount} total`}
              </span>
            </div>
          </m.div>
        )}
      </AnimatePresence>
    </div>
    </LazyMotion>
  );
}

function safeStringify(obj: unknown): string {
  try {
    const seen = new WeakSet();
    return JSON.stringify(obj, (_, value) => {
      if (typeof value === "object" && value !== null) {
        if (seen.has(value)) return "[Circular]";
        seen.add(value);
      }
      return value;
    });
  } catch {
    return String(obj);
  }
}
