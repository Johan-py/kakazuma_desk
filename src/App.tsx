import { useEffect } from "react";
import { Navbar } from "./components/Navbar";
import { HomeView } from "./components/HomeView";
import { CatalogView } from "./components/CatalogView";
import { SearchView } from "./components/SearchView";
import { FavoritesView } from "./components/FavoritesView";
import { HistoryView } from "./components/HistoryView";
import { DetailView } from "./components/DetailView";
import { PlayerOverlay } from "./components/PlayerOverlay";
import { useAppStore } from "./stores/useAppStore";

export default function App() {
  const view = useAppStore((s) => s.view);
  const initPlayer = useAppStore((s) => s.initPlayer);
  const initBuffer = useAppStore((s) => s.initBuffer);
  const loadTags = useAppStore((s) => s.loadTags);

  useEffect(() => {
    initPlayer();
    initBuffer();
    loadTags();
    useAppStore.getState().loadHome();
    useAppStore.getState().loadHistory();
    useAppStore.getState().loadFavorites();
  }, [initPlayer, initBuffer, loadTags]);

  return (
    <div className="min-h-full">
      <Navbar />
      <main>
        {view === "home" && <HomeView />}
        {view === "catalog" && <CatalogView />}
        {view === "search" && <SearchView />}
        {view === "favorites" && <FavoritesView />}
        {view === "history" && <HistoryView />}
        {view === "detail" && <DetailView />}
      </main>
      <PlayerOverlay />
    </div>
  );
}
