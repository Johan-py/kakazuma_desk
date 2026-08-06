import { useEffect } from "react";
import { useAppStore } from "../stores/useAppStore";
import { CarouselRow } from "./CarouselRow";

export function HomeView() {
  const recent = useAppStore((s) => s.recent);
  const recommended = useAppStore((s) => s.recommended);
  const history = useAppStore((s) => s.history);
  const homeLoading = useAppStore((s) => s.homeLoading);
  const loadHome = useAppStore((s) => s.loadHome);
  const openDetail = useAppStore((s) => s.openDetail);

  useEffect(() => {
    loadHome();
  }, [loadHome]);

  const hero = recommended[0] ?? recent[0];

  return (
    <div className="animate-fadein">
      {homeLoading && (
        <div className="flex h-40 items-center justify-center">
          <span className="h-8 w-8 animate-spin rounded-full border-2 border-primary border-t-transparent" />
        </div>
      )}

      {hero && (
        <div className="relative h-[55vh] min-h-[320px] w-full overflow-hidden">
          {hero.cover_image && (
            <img
              src={hero.cover_image}
              alt=""
              className="h-full w-full object-cover object-top"
            />
          )}
          <div className="absolute inset-0 bg-gradient-to-t from-bg via-bg/40 to-transparent" />
          <div className="absolute bottom-10 left-6 max-w-xl sm:left-10">
            <h1 className="text-3xl font-black sm:text-4xl">{hero.name}</h1>
            {hero.synopsis && (
              <p className="mt-2 line-clamp-3 text-sm text-muted">{hero.synopsis}</p>
            )}
            <button
              onClick={() => openDetail(hero.slug)}
              className="mt-4 rounded-md bg-primary px-6 py-2.5 text-sm font-semibold text-white hover:bg-primary-hover"
            >
              Ver detalles
            </button>
          </div>
        </div>
      )}

      {history.length > 0 && (
        <CarouselRow
          title="Continuar viendo"
          items={history.map((h) => h.anime)}
        />
      )}
      <CarouselRow title="Recomendados" items={recommended} />
      <CarouselRow title="Animes recientes" items={recent} />
    </div>
  );
}
