import { useCallback, useEffect, useRef, useState } from "react";
import Hls from "hls.js";
import { useAppStore } from "../stores/useAppStore";
import { api } from "../lib/api";

function fmt(t: number): string {
  if (!Number.isFinite(t) || t < 0) t = 0;
  const h = Math.floor(t / 3600);
  const m = Math.floor((t % 3600) / 60);
  const s = Math.floor(t % 60);
  const mm = String(m).padStart(2, "0");
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

function isHlsUrl(url: string): boolean {
  return /\.m3u8(\?|$)/i.test(url);
}

function isInputFocused(): boolean {
  const el = document.activeElement;
  if (!el) return false;
  const tag = el.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || (el as HTMLElement).isContentEditable;
}

const SPEEDS = [0.5, 0.75, 1, 1.25, 1.5, 2];

export function VideoPlayer() {
  const videoRef = useRef<HTMLVideoElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const hlsRef = useRef<Hls | null>(null);
  const saveTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const hideTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const currentUrlRef = useRef<string | null>(null);
  const lastStoreSyncRef = useRef(0);

  const player = useAppStore((s) => s.player);
  const setPlayerState = useAppStore((s) => s.setPlayerState);
  const consumeCommand = useAppStore((s) => s.consumeCommand);
  const stopPlayer = useAppStore((s) => s.stopPlayer);

  const [progress, setProgress] = useState(0);
  const [duration, setDuration] = useState(0);
  const [volume, setVolume] = useState(100);
  const [muted, setMuted] = useState(false);
  const [speed, setSpeed] = useState(1);
  const [controlsVisible, setControlsVisible] = useState(true);
  const [buffering, setBuffering] = useState(false);
  const [dragScrub, setDragScrub] = useState<number | null>(null);
  const [containerH, setContainerH] = useState(0);

  const videoUrl = player.videoUrl;

  useEffect(() => {
    const measure = () => {
      const w = window.innerWidth;
      const h = Math.min(Math.round((w * 9) / 16), Math.round(window.innerHeight * 0.7));
      setContainerH(h);
    };
    measure();
    window.addEventListener("resize", measure);
    return () => window.removeEventListener("resize", measure);
  }, []);

  const syncToBackend = useCallback(
    (overrides?: Record<string, unknown>) => {
      const v = videoRef.current;
      if (!v) return;
      setPlayerState({
        loaded: v.readyState >= 2,
        playing: !v.paused && !v.ended,
        position: v.currentTime,
        duration: isFinite(v.duration) ? v.duration : 0,
        muted: v.muted,
        buffering: v.readyState < 3,
        ...overrides,
      });
    },
    [setPlayerState]
  );

  // Throttled position sync to store (every 500ms max)
  const throttledSyncPosition = useCallback(() => {
    const now = Date.now();
    if (now - lastStoreSyncRef.current < 500) return;
    lastStoreSyncRef.current = now;
    const v = videoRef.current;
    if (!v) return;
    setPlayerState({
      position: v.currentTime,
      playing: !v.paused && !v.ended,
      duration: isFinite(v.duration) ? v.duration : 0,
    });
  }, [setPlayerState]);

  // Save progress every 10s
  useEffect(() => {
    if (!player.loaded) return;
    saveTimerRef.current = setInterval(() => {
      const v = videoRef.current;
      if (v && isFinite(v.duration) && v.duration > 0) {
        api.saveProgress(player.slug!, player.number, v.currentTime, v.duration);
        syncToBackend();
      }
    }, 10_000);
    return () => {
      if (saveTimerRef.current) clearInterval(saveTimerRef.current);
    };
  }, [player.loaded, player.slug, player.number, syncToBackend]);

  useEffect(() => {
    const handler = () => {
      const v = videoRef.current;
      if (v && player.slug && isFinite(v.duration) && v.duration > 0) {
        api.saveProgress(player.slug, player.number, v.currentTime, v.duration);
      }
    };
    window.addEventListener("beforeunload", handler);
    return () => window.removeEventListener("beforeunload", handler);
  }, [player.slug, player.number]);

  const showControls = useCallback(() => {
    setControlsVisible(true);
    if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
    hideTimerRef.current = setTimeout(() => {
      const v = videoRef.current;
      if (v && !v.paused) setControlsVisible(false);
    }, 3000);
  }, []);

  useEffect(() => () => {
    if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
  }, []);

  // Fix 5: Keyboard shortcuts with focus guard
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (!player.loaded) return;
      if (isInputFocused()) return;
      const v = videoRef.current;
      if (!v) return;
      switch (e.key) {
        case " ":
        case "k":
          e.preventDefault();
          v.paused ? v.play() : v.pause();
          showControls();
          break;
        case "ArrowLeft":
          e.preventDefault();
          v.currentTime = Math.max(0, v.currentTime - 10);
          showControls();
          break;
        case "ArrowRight":
          e.preventDefault();
          v.currentTime = Math.min(v.duration, v.currentTime + 10);
          showControls();
          break;
        case "ArrowUp":
          e.preventDefault();
          v.volume = Math.min(1, v.volume + 0.05);
          setVolume(Math.round(v.volume * 100));
          showControls();
          break;
        case "ArrowDown":
          e.preventDefault();
          v.volume = Math.max(0, v.volume - 0.05);
          setVolume(Math.round(v.volume * 100));
          showControls();
          break;
        case "f":
          e.preventDefault();
          toggleFullscreen();
          break;
        case "m":
          e.preventDefault();
          v.muted = !v.muted;
          setMuted(v.muted);
          showControls();
          break;
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [player.loaded, showControls]);

  // Fix 1: Process command queue from store (PlayerOverlay, etc.)
  useEffect(() => {
    const interval = setInterval(() => {
      const cmd = consumeCommand();
      if (!cmd) return;
      const v = videoRef.current;
      if (!v) return;
      switch (cmd.type) {
        case "play":
          v.play().catch(() => {});
          break;
        case "pause":
          v.pause();
          break;
        case "togglePause":
          v.paused ? v.play().catch(() => {}) : v.pause();
          break;
        case "seek":
          v.currentTime = cmd.position;
          break;
        case "setSpeed":
          v.playbackRate = cmd.speed;
          setSpeed(cmd.speed);
          break;
        case "setVolume":
          v.volume = cmd.volume / 100;
          setVolume(cmd.volume);
          break;
        case "setMute":
          v.muted = cmd.muted;
          setMuted(cmd.muted);
          break;
      }
    }, 50);
    return () => clearInterval(interval);
  }, [consumeCommand]);

  const toggleFullscreen = useCallback(() => {
    const c = containerRef.current;
    if (!c) return;
    if (document.fullscreenElement) {
      document.exitFullscreen();
    } else {
      c.requestFullscreen();
    }
  }, []);

  const handlePlayPause = useCallback(() => {
    const v = videoRef.current;
    if (!v) return;
    v.paused ? v.play() : v.pause();
  }, []);

  const handleSeek = useCallback((e: React.MouseEvent<HTMLDivElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    const v = videoRef.current;
    if (!v || !isFinite(v.duration)) return;
    const pct = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
    v.currentTime = pct * v.duration;
  }, []);

  const handleVolumeChange = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    const v = videoRef.current;
    if (!v) return;
    const vol = Number(e.target.value);
    v.volume = vol / 100;
    v.muted = vol === 0;
    setVolume(vol);
    setMuted(vol === 0);
  }, []);

  const handleSpeedChange = useCallback((e: React.ChangeEvent<HTMLSelectElement>) => {
    const v = videoRef.current;
    if (!v) return;
    const s = Number(e.target.value);
    v.playbackRate = s;
    setSpeed(s);
  }, []);

  const handleClose = useCallback(() => {
    if (hlsRef.current) {
      hlsRef.current.destroy();
      hlsRef.current = null;
    }
    const v = videoRef.current;
    if (v) {
      v.pause();
      v.removeAttribute("src");
      v.load();
    }
    if (saveTimerRef.current) clearInterval(saveTimerRef.current);
    currentUrlRef.current = null;
    stopPlayer();
  }, [stopPlayer]);

  // Sync speed/volume to video element without reinitializing
  useEffect(() => {
    const v = videoRef.current;
    if (!v) return;
    v.playbackRate = speed;
  }, [speed]);

  useEffect(() => {
    const v = videoRef.current;
    if (!v) return;
    v.volume = volume / 100;
  }, [volume]);

  // Initialize video — ONLY when URL changes
  useEffect(() => {
    const v = videoRef.current;
    if (!v || !videoUrl) return;
    if (currentUrlRef.current === videoUrl) return;
    currentUrlRef.current = videoUrl;

    if (hlsRef.current) {
      hlsRef.current.destroy();
      hlsRef.current = null;
    }

    v.removeAttribute("src");
    v.load();
    setProgress(0);
    setDuration(0);

    // Fix 2: play/pause listeners to keep store in sync
    const onPlay = () => syncToBackend();
    const onPause = () => syncToBackend();
    const onCanPlay = () => {
      syncToBackend({ loaded: true, error: null });
      if (player.position > 5) {
        v.currentTime = player.position;
      }
      v.play().catch(() => {});
    };
    // Fix 3: throttled timeupdate → store
    const onTimeUpdate = () => {
      setProgress(v.currentTime);
      throttledSyncPosition();
    };
    const onDurationChange = () => {
      if (isFinite(v.duration)) setDuration(v.duration);
    };
    const onWaiting = () => {
      setBuffering(true);
      syncToBackend({ buffering: true });
    };
    const onPlaying = () => {
      setBuffering(false);
      syncToBackend({ buffering: false });
      showControls();
    };
    const onEnded = () => {
      syncToBackend({ playing: false });
      setControlsVisible(true);
      api.saveProgress(
        player.slug!,
        player.number,
        v.currentTime,
        isFinite(v.duration) ? v.duration : 0
      );
    };
    const onError = () => {
      const err = v.error;
      const msg = err ? `Error ${err.code}: ${err.message}` : "Error al reproducir el video";
      syncToBackend({ error: msg });
    };

    v.addEventListener("play", onPlay);
    v.addEventListener("pause", onPause);
    v.addEventListener("canplay", onCanPlay);
    v.addEventListener("timeupdate", onTimeUpdate);
    v.addEventListener("durationchange", onDurationChange);
    v.addEventListener("waiting", onWaiting);
    v.addEventListener("playing", onPlaying);
    v.addEventListener("ended", onEnded);
    v.addEventListener("error", onError);

    if (isHlsUrl(videoUrl)) {
      if (Hls.isSupported()) {
        const hls = new Hls({
          enableWorker: true,
          lowLatencyMode: false,
          maxBufferLength: 300,
          maxMaxBufferLength: 600,
        });
        hlsRef.current = hls;
        hls.loadSource(videoUrl);
        hls.attachMedia(v);
        hls.on(Hls.Events.MANIFEST_PARSED, () => {
          v.play().catch(() => {});
        });
        hls.on(Hls.Events.ERROR, (_event, data) => {
          if (data.fatal) {
            switch (data.type) {
              case Hls.ErrorTypes.NETWORK_ERROR:
                hls.startLoad();
                break;
              case Hls.ErrorTypes.MEDIA_ERROR:
                hls.recoverMediaError();
                break;
              default:
                syncToBackend({ error: `HLS: ${data.details}` });
                break;
            }
          }
        });
      } else if (v.canPlayType("application/vnd.apple.mpegurl")) {
        v.src = videoUrl;
        v.load();
      } else {
        syncToBackend({ error: "HLS no soportado" });
      }
    } else {
      v.src = videoUrl;
      v.load();
    }

    return () => {
      v.removeEventListener("play", onPlay);
      v.removeEventListener("pause", onPause);
      v.removeEventListener("canplay", onCanPlay);
      v.removeEventListener("timeupdate", onTimeUpdate);
      v.removeEventListener("durationchange", onDurationChange);
      v.removeEventListener("waiting", onWaiting);
      v.removeEventListener("playing", onPlaying);
      v.removeEventListener("ended", onEnded);
      v.removeEventListener("error", onError);
      if (hlsRef.current) {
        hlsRef.current.destroy();
        hlsRef.current = null;
      }
    };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [videoUrl]);

  if (!videoUrl) return null;

  const pct = duration > 0 ? ((dragScrub ?? progress) / duration) * 100 : 0;

  return (
    <div
      ref={containerRef}
      style={{
        position: "relative",
        width: "100%",
        height: containerH || 450,
        backgroundColor: "#000",
        overflow: "hidden",
        flexShrink: 0,
      }}
      onMouseMove={showControls}
      onDoubleClick={toggleFullscreen}
    >
      <video
        ref={videoRef}
        width={containerH ? Math.round((containerH * 16) / 9) : 800}
        height={containerH || 450}
        preload="auto"
        playsInline
        onClick={handlePlayPause}
        style={{
          position: "absolute",
          top: 0,
          left: 0,
          width: "100%",
          height: "100%",
          objectFit: "contain",
          backgroundColor: "#000",
        }}
      />

      {buffering && (
        <div style={{ position: "absolute", inset: 0, display: "flex", alignItems: "center", justifyContent: "center", zIndex: 10 }}>
          <span className="h-12 w-12 animate-spin rounded-full border-4 border-white border-t-transparent" />
        </div>
      )}

      {player.error && (
        <div style={{ position: "absolute", inset: 0, display: "flex", alignItems: "center", justifyContent: "center", backgroundColor: "rgba(0,0,0,0.8)", zIndex: 10 }}>
          <div style={{ textAlign: "center" }}>
            <p style={{ color: "#E50914", fontSize: 14 }}>{player.error}</p>
            <button
              onClick={handleClose}
              style={{ marginTop: 12, padding: "8px 16px", backgroundColor: "#1F1F1F", color: "#fff", borderRadius: 6, fontSize: 12, cursor: "pointer" }}
            >
              Cerrar
            </button>
          </div>
        </div>
      )}

      <div
        style={{
          position: "absolute",
          bottom: 0,
          left: 0,
          right: 0,
          background: "linear-gradient(to top, rgba(0,0,0,0.9), rgba(0,0,0,0.4) 50%, transparent)",
          opacity: controlsVisible ? 1 : 0,
          pointerEvents: controlsVisible ? "auto" : "none",
          transition: "opacity 0.3s",
          zIndex: 20,
        }}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div style={{ padding: "32px 16px 4px" }}>
          <p style={{ color: "#fff", fontSize: 14, fontWeight: 600, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            {player.title ?? `Episodio ${player.number}`}
          </p>
        </div>

        <div
          style={{ margin: "0 16px", height: 6, borderRadius: 3, backgroundColor: "rgba(255,255,255,0.2)", cursor: "pointer" }}
          onClick={handleSeek}
          onMouseDown={(e) => {
            const rect = e.currentTarget.getBoundingClientRect();
            const v = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
            setDragScrub(v * duration);
            const onMove = (ev: MouseEvent) => {
              const v2 = Math.max(0, Math.min(1, (ev.clientX - rect.left) / rect.width));
              setDragScrub(v2 * duration);
            };
            const onUp = (ev: MouseEvent) => {
              const rect2 = e.currentTarget.getBoundingClientRect();
              const v3 = Math.max(0, Math.min(1, (ev.clientX - rect2.left) / rect2.width));
              const vid = videoRef.current;
              if (vid && isFinite(vid.duration)) {
                vid.currentTime = v3 * vid.duration;
              }
              setDragScrub(null);
              window.removeEventListener("mousemove", onMove);
              window.removeEventListener("mouseup", onUp);
            };
            window.addEventListener("mousemove", onMove);
            window.addEventListener("mouseup", onUp);
          }}
        >
          <div
            style={{
              height: "100%",
              borderRadius: 3,
              backgroundColor: "#E50914",
              width: `${Math.max(0, Math.min(100, pct))}%`,
              transition: dragScrub === null ? "width 0.1s" : "none",
            }}
          />
        </div>

        <p style={{ margin: "2px 16px 0", fontSize: 11, color: "rgba(255,255,255,0.6)", fontVariantNumeric: "tabular-nums" }}>
          {fmt(dragScrub ?? progress)} / {fmt(duration)}
        </p>

        <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "4px 16px 12px" }}>
          <button
            onClick={handlePlayPause}
            style={{ padding: "8px 16px", backgroundColor: "rgba(255,255,255,0.1)", color: "#fff", borderRadius: 6, fontSize: 14, fontWeight: 700, cursor: "pointer", border: "none" }}
          >
            {player.playing ? "⏸" : "▶"}
          </button>

          <button
            onClick={() => {
              const v = videoRef.current;
              if (!v) return;
              v.muted = !v.muted;
              setMuted(v.muted);
            }}
            style={{ padding: "8px 12px", backgroundColor: "transparent", color: "#fff", borderRadius: 6, fontSize: 14, cursor: "pointer", border: "none" }}
            title={muted ? "Desmutear" : "Mutear"}
          >
            {muted || volume === 0 ? "🔇" : volume < 50 ? "🔉" : "🔊"}
          </button>
          <input
            type="range"
            min={0}
            max={100}
            value={muted ? 0 : volume}
            onChange={handleVolumeChange}
            style={{ width: 80, accentColor: "#E50914" }}
          />

          <div style={{ flex: 1 }} />

          <select
            value={speed}
            onChange={handleSpeedChange}
            style={{ padding: "4px 8px", backgroundColor: "rgba(255,255,255,0.1)", color: "#fff", borderRadius: 6, fontSize: 12, border: "none", cursor: "pointer" }}
          >
            {SPEEDS.map((s) => (
              <option key={s} value={s}>{s}x</option>
            ))}
          </select>

          <button
            onClick={toggleFullscreen}
            style={{ padding: "8px 12px", backgroundColor: "transparent", color: "#fff", borderRadius: 6, fontSize: 14, cursor: "pointer", border: "none" }}
            title="Pantalla completa"
          >
            ⛶
          </button>

          <button
            onClick={handleClose}
            style={{ padding: "8px 12px", backgroundColor: "transparent", color: "rgba(255,255,255,0.6)", borderRadius: 6, fontSize: 14, cursor: "pointer", border: "none" }}
            title="Detener"
          >
            ✕
          </button>
        </div>
      </div>
    </div>
  );
}
