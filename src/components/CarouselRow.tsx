import type { Anime } from "../lib/types";
import { Poster } from "./Poster";
import { useAppStore } from "../stores/useAppStore";

export function CarouselRow({ title, items }: { title: string; items: Anime[] }) {
  const openDetail = useAppStore((s) => s.openDetail);
  if (items.length === 0) return null;
  return (
    <div className="mt-10">
      <h2 className="mb-3 px-6 text-lg font-bold sm:text-xl">{title}</h2>
      <div className="no-scrollbar flex gap-3 overflow-x-auto px-6 pb-2">
        {items.map((a) => (
          <div key={a.slug} className="w-36 shrink-0 sm:w-44">
            <Poster anime={a} onClick={() => openDetail(a.slug)} />
          </div>
        ))}
      </div>
    </div>
  );
}
