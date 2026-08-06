import { useAppStore, type View } from "../stores/useAppStore";

const TABS: { id: View; label: string }[] = [
  { id: "home", label: "Inicio" },
  { id: "catalog", label: "Filtros" },
  { id: "search", label: "Buscar" },
  { id: "favorites", label: "Favoritos" },
  { id: "history", label: "Historial" },
];

export function Navbar() {
  const view = useAppStore((s) => s.view);
  const setView = useAppStore((s) => s.setView);
  const setViewTo = (v: View) => {
    if (v === "home") useAppStore.getState().loadHome();
    if (v === "favorites") useAppStore.getState().loadFavorites();
    if (v === "history") useAppStore.getState().loadHistory();
    setView(v);
  };

  return (
    <header className="fixed top-0 z-40 w-full bg-gradient-to-b from-bg via-bg/90 to-transparent px-6 py-4">
      <div className="mx-auto flex max-w-[1400px] items-center justify-between">
        <button
          onClick={() => setViewTo("home")}
          className="text-2xl font-black tracking-tight"
          style={{ color: "#E50914" }}
        >
          KAKASUMA
        </button>
        <nav className="flex items-center gap-1">
          {TABS.map((t) => (
            <button
              key={t.id}
              onClick={() => setViewTo(t.id)}
              className={`rounded-md px-3 py-1.5 text-sm font-medium transition-colors ${
                view === t.id
                  ? "bg-surface text-text"
                  : "text-muted hover:bg-surface/60 hover:text-text"
              }`}
            >
              {t.label}
            </button>
          ))}
        </nav>
      </div>
    </header>
  );
}
