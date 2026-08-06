import { useEffect, useState } from "react";
import { useAppStore } from "../stores/useAppStore";
import { Poster } from "./Poster";

export function SearchView() {
  const searchQuery = useAppStore((s) => s.searchQuery);
  const setSearchQuery = useAppStore((s) => s.setSearchQuery);
  const searchResults = useAppStore((s) => s.searchResults);
  const searchLoading = useAppStore((s) => s.searchLoading);
  const runSearch = useAppStore((s) => s.runSearch);
  const openDetail = useAppStore((s) => s.openDetail);
  const [debounced, setDebounced] = useState(searchQuery);

  useEffect(() => {
    const t = setTimeout(() => setDebounced(searchQuery), 400);
    return () => clearTimeout(t);
  }, [searchQuery]);

  useEffect(() => {
    if (debounced.trim().length >= 2) runSearch(debounced.trim());
  }, [debounced, runSearch]);

  return (
    <div className="animate-fadein mx-auto max-w-[1400px] px-6 pt-24">
      <input
        value={searchQuery}
        onChange={(e) => setSearchQuery(e.target.value)}
        placeholder="Buscar anime…"
        autoFocus
        className="w-full max-w-xl rounded-lg bg-surface px-4 py-3 text-text placeholder-muted focus:outline-none focus:ring-2 focus:ring-primary"
      />
      {searchLoading && (
        <div className="mt-12 flex justify-center">
          <span className="h-8 w-8 animate-spin rounded-full border-2 border-primary border-t-transparent" />
        </div>
      )}
      {!searchLoading && searchResults.length === 0 && searchQuery.trim() !== "" && (
        <p className="mt-12 text-center text-muted">Sin resultados para «{searchQuery}».</p>
      )}
      <div className="mt-8 grid grid-cols-3 gap-3 sm:grid-cols-4 md:grid-cols-5 lg:grid-cols-6 xl:grid-cols-7">
        {searchResults.map((a) => (
          <Poster key={a.slug} anime={a} onClick={() => openDetail(a.slug)} />
        ))}
      </div>
    </div>
  );
}
