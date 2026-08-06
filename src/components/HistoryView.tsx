import { useEffect } from "react";
import { useAppStore } from "../stores/useAppStore";
import { Poster } from "./Poster";

export function HistoryView() {
  const history = useAppStore((s) => s.history);
  const historyLoading = useAppStore((s) => s.historyLoading);
  const loadHistory = useAppStore((s) => s.loadHistory);
  const clearHistory = useAppStore((s) => s.clearHistory);
  const openDetail = useAppStore((s) => s.openDetail);

  useEffect(() => {
    loadHistory();
  }, [loadHistory]);

  if (historyLoading && history.length === 0) {
    return (
      <div className="flex h-64 items-center justify-center pt-24">
        <span className="h-8 w-8 animate-spin rounded-full border-2 border-primary border-t-transparent" />
      </div>
    );
  }

  if (history.length === 0) {
    return (
      <p className="pt-40 text-center text-muted">
        Nada por aquí todavía. Cuando veas un episodio aparecerá en tu historial.
      </p>
    );
  }

  return (
    <div className="mx-auto max-w-[1400px] px-6 pt-24">
      <div className="mb-6 flex items-center justify-between">
        <h1 className="text-xl font-bold">Historial</h1>
        <button
          onClick={clearHistory}
          className="rounded-md bg-surface px-3 py-1.5 text-sm text-muted hover:text-primary"
        >
          Vaciar historial
        </button>
      </div>
      <div className="grid grid-cols-3 gap-3 sm:grid-cols-4 md:grid-cols-5 lg:grid-cols-6 xl:grid-cols-7">
        {history.map((h) => (
          <Poster key={h.anime.slug} anime={h.anime} onClick={() => openDetail(h.anime.slug)} />
        ))}
      </div>
    </div>
  );
}
