import { useEffect } from "react";
import { useAppStore } from "../stores/useAppStore";
import { Poster } from "./Poster";

const GENEROS = [
  "accion", "aventura", "comedia", "drama", "ecchi", "fantasia", "isekai", "misterio",
  "romance", "sci-fi", "shounen", "seinen", "shoujo", "suspenso", "terror", "deportes",
  "magia", "mecha", "musica", "sobrenatural", "psicologico", "policial", "militar",
];

const TIPOS = ["animes", "peliculas", "especiales", "ovas", "onas"];
const ESTADOS = ["emision", "finalizados", "estrenos"];
const ORDEN = ["nombre", "popularidad"];

export function CatalogView() {
  const filter = useAppStore((s) => s.filter);
  const setFilter = useAppStore((s) => s.setFilter);
  const resetFilter = useAppStore((s) => s.resetFilter);
  const catalog = useAppStore((s) => s.catalog);
  const catalogLoading = useAppStore((s) => s.catalogLoading);
  const loadCatalog = useAppStore((s) => s.loadCatalog);
  const openDetail = useAppStore((s) => s.openDetail);

  useEffect(() => {
    loadCatalog(1);
  }, [loadCatalog]);

  const selectCls =
    "rounded-md bg-surface px-3 py-1.5 text-sm text-text focus:outline-none focus:ring-1 focus:ring-primary";

  return (
    <div className="animate-fadein mx-auto max-w-[1400px] px-6 pt-24">
      <div className="flex flex-wrap items-center gap-3">
        <select
          className={selectCls}
          value={filter.genero ?? ""}
          onChange={(e) => setFilter({ genero: e.target.value || null })}
        >
          <option value="">Género</option>
          {GENEROS.map((g) => (
            <option key={g} value={g}>{g}</option>
          ))}
        </select>
        <select
          className={selectCls}
          value={filter.tipo ?? ""}
          onChange={(e) => setFilter({ tipo: e.target.value || null })}
        >
          <option value="">Tipo</option>
          {TIPOS.map((t) => (
            <option key={t} value={t}>{t}</option>
          ))}
        </select>
        <select
          className={selectCls}
          value={filter.estado ?? ""}
          onChange={(e) => setFilter({ estado: e.target.value || null })}
        >
          <option value="">Estado</option>
          {ESTADOS.map((e) => (
            <option key={e} value={e}>{e}</option>
          ))}
        </select>
        <select
          className={selectCls}
          value={filter.orden ?? ""}
          onChange={(e) => setFilter({ orden: e.target.value || null })}
        >
          <option value="">Orden</option>
          {ORDEN.map((o) => (
            <option key={o} value={o}>{o}</option>
          ))}
        </select>
        <button
          onClick={resetFilter}
          className="rounded-md bg-surface px-3 py-1.5 text-sm font-medium text-muted hover:text-text"
        >
          Limpiar
        </button>
      </div>

      {catalogLoading && catalog == null && (
        <div className="mt-16 flex justify-center">
          <span className="h-8 w-8 animate-spin rounded-full border-2 border-primary border-t-transparent" />
        </div>
      )}

      {catalog && (
        <>
          <div className="mt-6 grid grid-cols-3 gap-3 sm:grid-cols-4 md:grid-cols-5 lg:grid-cols-6 xl:grid-cols-7">
            {catalog.items.map((a) => (
              <Poster key={a.slug} anime={a} onClick={() => openDetail(a.slug)} />
            ))}
          </div>
          {catalog.last_page > 1 && (
            <div className="mt-6 flex items-center justify-center gap-2 pb-10">
              <button
                disabled={catalog.page <= 1}
                onClick={() => loadCatalog(catalog.page - 1)}
                className="rounded-md bg-surface px-4 py-1.5 text-sm disabled:opacity-40"
              >
                ← Anterior
              </button>
              <span className="text-sm text-muted">
                {catalog.page} / {catalog.last_page}
              </span>
              <button
                disabled={catalog.page >= catalog.last_page}
                onClick={() => loadCatalog(catalog.page + 1)}
                className="rounded-md bg-surface px-4 py-1.5 text-sm disabled:opacity-40"
              >
                Siguiente →
              </button>
            </div>
          )}
        </>
      )}
    </div>
  );
}
