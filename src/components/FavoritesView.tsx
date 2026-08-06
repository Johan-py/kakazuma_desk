import { useEffect } from "react";
import { useAppStore } from "../stores/useAppStore";
import { Poster } from "./Poster";

export function FavoritesView() {
  const favorites = useAppStore((s) => s.favorites);
  const favoritesLoading = useAppStore((s) => s.favoritesLoading);
  const loadFavorites = useAppStore((s) => s.loadFavorites);
  const openDetail = useAppStore((s) => s.openDetail);

  useEffect(() => {
    loadFavorites();
  }, [loadFavorites]);

  if (favoritesLoading && favorites.length === 0) {
    return (
      <div className="flex h-64 items-center justify-center pt-24">
        <span className="h-8 w-8 animate-spin rounded-full border-2 border-primary border-t-transparent" />
      </div>
    );
  }

  if (favorites.length === 0) {
    return (
      <p className="pt-40 text-center text-muted">
        Aún no tienes favoritos. Marca animes con ♥ para verlos aquí.
      </p>
    );
  }

  return (
    <div className="mx-auto max-w-[1400px] px-6 pt-24">
      <h1 className="mb-6 text-xl font-bold">Mis favoritos</h1>
      <div className="grid grid-cols-3 gap-3 sm:grid-cols-4 md:grid-cols-5 lg:grid-cols-6 xl:grid-cols-7">
        {favorites.map((a) => (
          <Poster key={a.slug} anime={a} onClick={() => openDetail(a.slug)} />
        ))}
      </div>
    </div>
  );
}
