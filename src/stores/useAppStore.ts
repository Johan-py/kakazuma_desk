import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { api } from "../lib/api";
import type {
  Anime,
  AnimeDetail,
  CatalogFilter,
  CatalogPage,
  PlayerProgressEvent,
  PlayerState,
  Tag,
  WatchHistoryEntry,
} from "../lib/types";

export type View = "home" | "catalog" | "search" | "favorites" | "history" | "detail";

export const DEFAULT_FILTER: CatalogFilter = {
  genero: null,
  demografia: null,
  temporada: null,
  tipo: null,
  estado: null,
  anio: null,
  orden: null,
};

interface AppStore {
  view: View;
  setView: (v: View) => void;

  detail: AnimeDetail | null;
  detailLoading: boolean;
  detailError: string | null;
  openDetail: (slug: string) => Promise<void>;
  closeDetail: () => void;

  recent: Anime[];
  recommended: Anime[];
  homeLoading: boolean;
  loadHome: () => Promise<void>;

  tags: Tag[];
  loadTags: () => Promise<void>;

  filter: CatalogFilter;
  setFilter: (patch: Partial<CatalogFilter>) => void;
  resetFilter: () => void;
  catalog: CatalogPage | null;
  catalogPage: number;
  catalogLoading: boolean;
  loadCatalog: (page?: number) => Promise<void>;

  searchQuery: string;
  setSearchQuery: (q: string) => void;
  searchResults: Anime[];
  searchLoading: boolean;
  runSearch: (query: string) => Promise<void>;

  favorites: Anime[];
  favoritesLoading: boolean;
  loadFavorites: () => Promise<void>;
  toggleFavorite: (slug: string) => Promise<boolean>;

  history: WatchHistoryEntry[];
  historyLoading: boolean;
  loadHistory: () => Promise<void>;
  clearHistory: () => Promise<void>;

  player: PlayerState;
  playerVisible: boolean;
  initPlayer: () => Promise<void>;
  refreshPlayer: () => Promise<void>;
}

export const useAppStore = create<AppStore>((set, get) => ({
  view: "home",
  setView: (v) => set({ view: v }),

  detail: null,
  detailLoading: false,
  detailError: null,
  openDetail: async (slug) => {
    set({ detailLoading: true, detailError: null, view: "detail" });
    try {
      const detail = await api.getAnimeDetail(slug);
      set({ detail, detailLoading: false });
    } catch (e) {
      set({ detailError: (e as Error).message, detailLoading: false });
    }
  },
  closeDetail: () => set({ detail: null, detailError: null }),

  recent: [],
  recommended: [],
  homeLoading: false,
  loadHome: async () => {
    if (get().homeLoading) return;
    set({ homeLoading: true });
    try {
      const [recent, recommended] = await Promise.all([api.getRecent(), api.getRecommended()]);
      set({ recent, recommended, homeLoading: false });
    } catch (e) {
      console.error(e);
      set({ homeLoading: false });
    }
  },

  tags: [],
  loadTags: async () => {
    try {
      const tags = await api.getTags();
      set({ tags });
    } catch (e) {
      console.error(e);
    }
  },

  filter: { ...DEFAULT_FILTER },
  setFilter: (patch) => {
    set({ filter: { ...get().filter, ...patch } });
    get().loadCatalog(1);
  },
  resetFilter: () => {
    set({ filter: { ...DEFAULT_FILTER } });
    get().loadCatalog(1);
  },
  catalog: null,
  catalogPage: 1,
  catalogLoading: false,
  loadCatalog: async (page = get().catalogPage) => {
    set({ catalogLoading: true, catalogPage: page });
    try {
      const res = await api.getCatalog(get().filter, page);
      set({ catalog: res, catalogLoading: false });
    } catch (e) {
      console.error(e);
      set({ catalogLoading: false });
    }
  },

  searchQuery: "",
  setSearchQuery: (q) => set({ searchQuery: q }),
  searchResults: [],
  searchLoading: false,
  runSearch: async (query) => {
    set({ searchLoading: true });
    try {
      const res = await api.searchAnime(query);
      set({ searchResults: res, searchLoading: false });
    } catch (e) {
      console.error(e);
      set({ searchResults: [], searchLoading: false });
    }
  },

  favorites: [],
  favoritesLoading: false,
  loadFavorites: async () => {
    set({ favoritesLoading: true });
    try {
      const entries = await api.listFavorites();
      set({ favorites: entries.map((e) => e.anime), favoritesLoading: false });
    } catch (e) {
      console.error(e);
      set({ favoritesLoading: false });
    }
  },
  toggleFavorite: async (slug) => {
    try {
      const isFav = await api.isFavorite(slug);
      if (isFav) {
        await api.removeFavorite(slug);
      } else {
        await api.addFavorite(slug);
      }
      await get().loadFavorites();
      return !isFav;
    } catch (e) {
      console.error(e);
      return get().favorites.some((a) => a.slug === slug);
    }
  },

  history: [],
  historyLoading: false,
  loadHistory: async () => {
    set({ historyLoading: true });
    try {
      const res = await api.continueWatching();
      set({ history: res, historyLoading: false });
    } catch (e) {
      console.error(e);
      set({ historyLoading: false });
    }
  },
  clearHistory: async () => {
    try {
      await api.clearHistory();
      set({ history: [] });
    } catch (e) {
      console.error(e);
    }
  },

  player: {
    loaded: false,
    playing: false,
    position: 0,
    duration: 0,
    speed: 1,
    volume: 100,
    muted: false,
    fullscreen: false,
    title: null,
    slug: null,
    number: 0,
    error: null,
  },
  playerVisible: false,
  initPlayer: async () => {
    await get().refreshPlayer();
    const onState = await listen<PlayerState>("player://state", (e) => {
      set({ player: e.payload });
    });
    const onProgress = await listen<PlayerProgressEvent>("player://progress", () => {
      get().refreshPlayer();
    });
    const onEnd = await listen<PlayerProgressEvent>("player://end", () => {
      const p = { ...get().player, playing: false };
      set({ player: p });
      get().loadHistory();
    });
    const onError = await listen<string>("player://error", (e) => {
      set({ player: { ...get().player, error: e.payload } });
    });
    window.__unlisteners = [onState, onProgress, onEnd, onError];
  },
  refreshPlayer: async () => {
    try {
      const state = await api.playerGetState();
      set({ player: state, playerVisible: state.loaded });
    } catch {
      // sin reproductor disponible
    }
  },
}));
