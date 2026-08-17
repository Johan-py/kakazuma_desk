import { useEffect } from "react";
import { Navbar } from "./components/Navbar";
import { HomeView } from "./components/HomeView";
import { CatalogView } from "./components/CatalogView";
import { SearchView } from "./components/SearchView";
import { FavoritesView } from "./components/FavoritesView";
import { HistoryView } from "./components/HistoryView";
import { DetailView } from "./components/DetailView";
import { SettingsView } from "./components/SettingsView";
import { PlayerOverlay } from "./components/PlayerOverlay";
import { VideoPlayer } from "./components/VideoPlayer";
import { useAppStore } from "./stores/useAppStore";

export default function App() {
  const view = useAppStore((s) => s.view);
  const player = useAppStore((s) => s.player);
  const initPlayer = useAppStore((s) => s.initPlayer);
  const initBuffer = useAppStore((s) => s.initBuffer);
  const loadTags = useAppStore((s) => s.loadTags);

  useEffect(() => {
    initPlayer();
    initBuffer();
    loadTags();
    useAppStore.getState().loadProvider();
    useAppStore.getState().loadHome();
    useAppStore.getState().loadHistory();
    useAppStore.getState().loadFavorites();
  }, [initPlayer, initBuffer, loadTags]);

  const showVideoPlayer = !!player.videoUrl;

  return (
    <div className="min-h-full">
      <Navbar />
      <div style={{ display: showVideoPlayer && view === "detail" ? "block" : "none" }}>
        <VideoPlayer />
      </div>
      <main>
        {view === "home" && <HomeView />}
        {view === "catalog" && <CatalogView />}
        {view === "search" && <SearchView />}
        {view === "favorites" && <FavoritesView />}
        {view === "history" && <HistoryView />}
        {view === "detail" && <DetailView />}
        {view === "settings" && <SettingsView />}
      </main>
      <PlayerOverlay />
    </div>
  );
}
