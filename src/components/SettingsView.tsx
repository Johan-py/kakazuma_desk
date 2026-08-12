import { useEffect } from "react";
import { useAppStore } from "../stores/useAppStore";

export function SettingsView() {
  const provider = useAppStore((s) => s.provider);
  const providerLoading = useAppStore((s) => s.providerLoading);
  const loadProvider = useAppStore((s) => s.loadProvider);
  const setProvider = useAppStore((s) => s.setProvider);

  useEffect(() => {
    loadProvider();
  }, [loadProvider]);

  return (
    <div className="animate-fadein mx-auto max-w-[1400px] px-6 pt-24">
      <h1 className="text-xl font-bold">Ajustes</h1>
      <p className="mt-1 text-sm text-muted">
        Configura la fuente de contenido que usa Kakazuma por defecto.
      </p>

      <div className="mt-8 max-w-xl rounded-lg border border-surface/60 bg-surface/40 p-6">
        <label className="block text-sm font-medium text-text">
          Fuente por defecto
        </label>
        <p className="mt-1 text-xs text-muted">
          Toda la búsqueda, el catálogo y la reproducción se resolverán con la
          fuente elegida.
        </p>
        <div className="mt-3 flex items-center gap-2">
          <select
            className="rounded-md bg-surface px-3 py-2 text-sm text-text focus:outline-none focus:ring-1 focus:ring-primary"
            disabled={providerLoading || !provider}
            value={provider?.current ?? ""}
            onChange={(e) => setProvider(e.target.value)}
          >
            {provider?.available.map((opt) => (
              <option key={opt.key} value={opt.key}>
                {opt.name}
              </option>
            ))}
          </select>
          {providerLoading && (
            <span className="h-4 w-4 animate-spin rounded-full border-2 border-primary border-t-transparent" />
          )}
        </div>
        <p className="mt-3 text-xs text-muted">
          {provider?.current === "latanime"
            ? "Latanime ofrece doblaje latino/castellano y streams MP4 directos."
            : "Jkanime es la fuente clásica con streams HLS."}
        </p>
      </div>
    </div>
  );
}