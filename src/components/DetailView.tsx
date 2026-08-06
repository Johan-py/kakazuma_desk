import { useEffect, useMemo, useState } from "react";
import { api } from "../lib/api";
import { useAppStore } from "../stores/useAppStore";
import type { Episode } from "../lib/types";

export function DetailView() {
  const detail = useAppStore((s) => s.detail);
  const detailLoading = useAppStore((s) => s.detailLoading);
  const detailError = useAppStore((s) => s.detailError);
  const closeDetail = useAppStore((s) => s.closeDetail);
  const toggleFavorite = useAppStore((s) => s.toggleFavorite);
  const history = useAppStore((s) => s.history);
  const player = useAppStore((s) => s.player);
  const [isFav, setIsFav] = useState(false);
  const [playing, setPlaying] = useState<number | null>(null);

  const slug = detail?.anime.slug;

  useEffect(() => {
    if (!slug) return;
    api.isFavorite(slug).then(setIsFav).catch(() => setIsFav(false));
  }, [slug]);

  const resumeFor = useMemo(() => {
    const map = new Map<string, { position: number; duration: number; number: number }>();
    for (const h of history) {
      if (h.anime.slug === slug && h.episode) {
        map.set(h.episode.number.toString(), {
          position: h.playback_position,
          duration: h.duration,
          number: h.episode.number,
        });
      }
    }
    return map;
  }, [history, slug]);

  const handlePlay = async (ep: Episode) => {
    setPlaying(ep.number);
    const resume = resumeFor.get(ep.number.toString());
    try {
      await api.playEpisode(slug!, ep.number, ep.title ?? `Episodio ${ep.number}`, resume?.position ?? 0);
    } catch (e) {
      console.error(e);
    } finally {
      setPlaying(null);
    }
  };

  if (detailLoading) {
    return (
      <div className="flex h-64 items-center justify-center pt-24">
        <span className="h-8 w-8 animate-spin rounded-full border-2 border-primary border-t-transparent" />
      </div>
    );
  }

  if (detailError || !detail) {
    return (
      <div className="px-6 pt-40 text-center">
        <p className="text-muted">{detailError ?? "No hay datos."}</p>
        <button
          onClick={closeDetail}
          className="mt-4 rounded-md bg-surface px-4 py-2 text-sm"
        >
          Volver
        </button>
      </div>
    );
  }

  const anime = detail.anime;
  const lastViewed = resumeFor.size > 0
    ? [...resumeFor.values()].sort((a, b) => b.position / b.duration - a.position / a.duration)[0]
    : null;

  return (
    <div className="animate-fadein pb-16 pt-20">
      <div className="relative h-[46vh] min-h-[280px] w-full overflow-hidden">
        {anime.cover_image && (
          <img src={anime.cover_image} alt="" className="h-full w-full object-cover object-top" />
        )}
        <div className="absolute inset-0 bg-gradient-to-t from-bg via-bg/50 to-transparent" />
        <div className="absolute bottom-6 left-6 max-w-2xl sm:left-10">
          <div className="flex flex-wrap items-center gap-2">
            <h1 className="text-3xl font-black sm:text-4xl">{anime.name}</h1>
            <button
              onClick={() => {
                if (slug) {
                  const next = !isFav;
                  setIsFav(next);
                  toggleFavorite(slug);
                }
              }}
              className={`rounded-full px-3 py-1.5 text-sm font-bold transition-colors ${
                isFav ? "bg-primary text-white" : "bg-surface text-muted hover:text-text"
              }`}
              title={isFav ? "Quitar de favoritos" : "Añadir a favoritos"}
            >
              {isFav ? "♥" : "♡"}
            </button>
          </div>
          <p className="mt-2 flex flex-wrap gap-2 text-xs text-muted">
            {anime.status && <span>{anime.status}</span>}
            {anime.anime_type && <span>· {anime.anime_type}</span>}
            {anime.total_episodes != null && <span>· {anime.total_episodes} ep.</span>}
            {anime.season && <span>· {anime.season}</span>}
          </p>
          <p className="mt-2 flex flex-wrap gap-1.5">
            {anime.genres.map((g) => (
              <span key={g} className="rounded bg-surface px-2 py-0.5 text-[11px] text-muted">
                {g}
              </span>
            ))}
          </p>
        </div>
      </div>

      {anime.synopsis && (
        <p className="mx-auto mt-6 max-w-[1400px] px-6 text-sm leading-relaxed text-muted">
          {anime.synopsis}
        </p>
      )}

      {lastViewed && (
        <div className="mx-auto mt-4 max-w-[1400px] px-6">
          <button
            onClick={() => {
              const ep = detail.episodes.find((e) => e.number === lastViewed.number);
              if (ep) handlePlay(ep);
            }}
            className="rounded-md bg-primary px-6 py-2.5 text-sm font-semibold text-white hover:bg-primary-hover"
          >
            ▶ Continuar episodio {lastViewed.number}
          </button>
        </div>
      )}

      <div className="mx-auto mt-8 max-w-[1400px] px-6">
        <h2 className="mb-3 text-lg font-bold">Episodios ({detail.episodes.length})</h2>
        {detail.episodes.length === 0 && (
          <p className="text-sm text-muted">No hay episodios disponibles.</p>
        )}
        <div className="grid grid-cols-2 gap-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-6">
          {detail.episodes
            .slice()
            .sort((a, b) => a.number - b.number)
            .map((ep) => {
              const prog = resumeFor.get(ep.number.toString());
              const pct = prog && prog.duration > 0 ? Math.min(100, (prog.position / prog.duration) * 100) : 0;
              const isCurrent = player.slug === slug && player.number === ep.number;
              return (
                <button
                  key={ep.number}
                  onClick={() => handlePlay(ep)}
                  disabled={playing === ep.number}
                  className={`relative overflow-hidden rounded-lg bg-surface px-3 py-3 text-left text-sm transition-colors hover:bg-surface-hover ${
                    isCurrent ? "ring-1 ring-primary" : ""
                  }`}
                >
                  <span className="font-semibold">Ep. {ep.number}</span>
                  {ep.title && <span className="ml-1 text-xs text-muted">· {ep.title}</span>}
                  {pct > 0 && (
                    <span className="absolute inset-x-0 bottom-0 h-0.5 bg-white/10">
                      <span className="block h-full bg-primary" style={{ width: `${pct}%` }} />
                    </span>
                  )}
                  {playing === ep.number && (
                    <span className="absolute right-2 top-2 h-4 w-4 animate-spin rounded-full border-2 border-primary border-t-transparent" />
                  )}
                </button>
              );
            })}
        </div>
      </div>
    </div>
  );
}
