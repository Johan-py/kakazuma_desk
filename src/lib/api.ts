import { invoke } from "@tauri-apps/api/core";
import type {
  Anime,
  AnimeDetail,
  BufferConfig,
  BufferStatus,
  CatalogFilter,
  CatalogPage,
  FavoriteEntry,
  ProviderInfo,
  Tag,
  VideoSource,
  WatchHistoryEntry,
} from "./types";

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw new Error(typeof e === "string" ? e : "Error desconocido");
  }
}

export const api = {
  searchAnime: (query: string) => call<Anime[]>("search_anime", { query }),

  getProvider: () => call<ProviderInfo>("get_provider"),

  setProvider: (provider: string) =>
    call<ProviderInfo>("set_provider", { provider }),

  getAnimeDetail: (slug: string) => call<AnimeDetail>("get_anime_detail", { slug }),

  getCatalog: (filter: CatalogFilter, page: number) =>
    call<CatalogPage>("get_catalog", { filter, page }),

  getTags: () => call<Tag[]>("get_tags"),

  getRecent: () => call<Anime[]>("get_recent"),

  getRecommended: () => call<Anime[]>("get_recommended"),

  resolveVideo: (slug: string, number: number) =>
    call<VideoSource>("resolve_video", { slug, number }),

  listFavorites: () => call<FavoriteEntry[]>("list_favorites"),

  addFavorite: (slug: string) => call<boolean>("add_favorite", { slug }),

  removeFavorite: (slug: string) => call<void>("remove_favorite", { slug }),

  isFavorite: (slug: string) => call<boolean>("is_favorite", { slug }),

  continueWatching: () => call<WatchHistoryEntry[]>("continue_watching"),

  saveProgress: (slug: string, episodeNumber: number | null, position: number, duration: number) =>
    call<void>("save_progress", { slug, episodeNumber, position, duration }),

  clearHistory: () => call<void>("clear_history"),

  playEpisode: (slug: string, number: number, title: string, start: number) =>
    call<VideoSource>("play_episode", { slug, number, title, start }),

  updatePlayerState: (
    slug: string | null,
    number: number,
    loaded: boolean,
    playing: boolean,
    position: number,
    duration: number,
    buffering: boolean
  ) =>
    call<void>("update_player_state", {
      slug,
      number,
      loaded,
      playing,
      position,
      duration,
      buffering,
    }),

  bufferGetConfig: () => call<BufferConfig>("buffer_get_config"),
  bufferSetConfig: (config: BufferConfig) => call<BufferConfig>("buffer_set_config", { config }),
  bufferGetStatus: () => call<BufferStatus>("buffer_get_status"),
  bufferClearCache: () => call<number>("buffer_clear_cache"),
  bufferPause: (paused: boolean) => call<void>("buffer_pause", { paused }),
};
