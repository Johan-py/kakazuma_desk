import type { Anime } from "../lib/types";

interface PosterProps {
  anime: Anime;
  onClick?: () => void;
}

export function Poster({ anime, onClick }: PosterProps) {
  return (
    <button
      onClick={onClick}
      className="group relative block w-full overflow-hidden rounded-xl bg-surface text-left transition-transform duration-200 hover:scale-[1.03] hover:shadow-xl hover:shadow-black/40 focus:outline-none"
      style={{ aspectRatio: "2/3" }}
    >
      {anime.cover_image ? (
        <img
          src={anime.cover_image}
          alt={anime.name}
          loading="lazy"
          className="h-full w-full object-cover"
        />
      ) : (
        <div className="flex h-full w-full items-center justify-center bg-surface">
          <span className="px-2 text-center text-sm font-semibold text-muted">
            {anime.name}
          </span>
        </div>
      )}
      <div className="absolute inset-x-0 bottom-0 bg-gradient-to-t from-black/90 to-transparent p-3 pt-8 opacity-0 transition-opacity group-hover:opacity-100">
        <p className="line-clamp-2 text-xs font-semibold text-white">{anime.name}</p>
        {anime.total_episodes != null && (
          <p className="mt-1 text-[11px] text-muted">{anime.total_episodes} ep.</p>
        )}
      </div>
      {anime.status && (
        <span className="absolute left-2 top-2 rounded bg-primary px-1.5 py-0.5 text-[10px] font-bold uppercase text-white">
          {anime.status}
        </span>
      )}
    </button>
  );
}
