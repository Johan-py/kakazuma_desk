import { useEffect, useState } from "react";
import { useAppStore } from "../stores/useAppStore";

function fmt(t: number): string {
  if (!Number.isFinite(t) || t < 0) t = 0;
  const h = Math.floor(t / 3600);
  const m = Math.floor((t % 3600) / 60);
  const s = Math.floor(t % 60);
  const mm = String(m).padStart(2, "0");
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

export function PlayerOverlay() {
  const player = useAppStore((s) => s.player);
  const view = useAppStore((s) => s.view);
  const sendCommand = useAppStore((s) => s.sendCommand);
  const [drag, setDrag] = useState(false);
  const [scrub, setScrub] = useState(0);

  const isDetail = view === "detail";
  const visible = player.loaded && player.videoUrl && !isDetail;

  useEffect(() => {
    if (!drag) setScrub(player.position);
  }, [drag, player.position]);

  if (!visible) return null;

  const pct = player.duration > 0 ? (drag ? scrub : player.position) / player.duration : 0;

  return (
    <div className="fixed inset-x-0 bottom-0 z-50 border-t border-surface-hover bg-bg/95 px-4 py-3 backdrop-blur">
      <div className="mx-auto flex max-w-[1400px] items-center gap-4">
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-semibold">
            {player.title ?? `Episodio ${player.number}`}
          </p>
          <div
            className="group mt-2 h-1.5 w-full cursor-pointer rounded-full bg-surface-hover"
            onMouseDown={(e) => {
              setDrag(true);
              const rect = e.currentTarget.getBoundingClientRect();
              const v = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
              setScrub(v * player.duration);
            }}
            onMouseMove={(e) => {
              if (!drag) return;
              const rect = e.currentTarget.getBoundingClientRect();
              const v = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
              setScrub(v * player.duration);
            }}
            onMouseUp={() => {
              if (drag) {
                sendCommand({ type: "seek", position: scrub });
                setDrag(false);
              }
            }}
            onMouseLeave={() => setDrag(false)}
          >
            <div
              className="h-full rounded-full bg-primary"
              style={{ width: `${Math.max(0, Math.min(1, pct)) * 100}%` }}
            />
          </div>
          <p className="mt-1 text-[11px] tabular-nums text-muted">
            {fmt(drag ? scrub : player.position)} / {fmt(player.duration)}
          </p>
        </div>

        <div className="flex items-center gap-2">
          <button
            onClick={() => sendCommand({ type: "togglePause" })}
            className="rounded-md bg-primary px-4 py-2 text-sm font-bold text-white hover:bg-primary-hover"
          >
            {player.playing ? "⏸" : "▶"}
          </button>
          <button
            onClick={() => sendCommand({ type: "pause" })}
            className="rounded-md bg-surface px-3 py-2 text-sm text-muted hover:text-primary"
            title="Detener"
          >
            ✕
          </button>
        </div>
      </div>
    </div>
  );
}
