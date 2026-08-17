export interface Anime {
  id: number;
  slug: string;
  name: string;
  synopsis: string | null;
  season: string | null;
  status: string | null;
  cover_image: string | null;
  total_episodes: number | null;
  anime_type: string | null;
  url: string;
  genres: string[];
}

export interface Episode {
  id: number;
  anime_id: number;
  number: number;
  title: string | null;
  video_url: string | null;
  duration: number | null;
}

export interface Tag {
  id: number;
  name: string;
  description: string | null;
}

export interface Subtitle {
  lang: string;
  url: string;
}

export interface VideoSource {
  url: string;
  quality: string | null;
  subtitles: Subtitle[];
}

export interface AnimeDetail {
  anime: Anime;
  episodes: Episode[];
  tags: Tag[];
}

export interface WatchHistoryEntry {
  id: number;
  anime: Anime;
  episode: Episode | null;
  playback_position: number;
  duration: number;
  date_first_view: number;
  date_last_view: number;
}

export interface FavoriteEntry {
  id: number;
  anime: Anime;
  date_added: number;
}

export interface CatalogFilter {
  genero: string | null;
  demografia: string | null;
  temporada: string | null;
  tipo: string | null;
  estado: string | null;
  anio: number | null;
  orden: string | null;
}

export interface CatalogPage {
  items: Anime[];
  page: number;
  total: number;
  per_page: number;
  last_page: number;
}

export interface ProviderOption {
  key: string;
  name: string;
}

export interface ProviderInfo {
  current: string;
  available: ProviderOption[];
}

export type PlayerPhase =
  | "idle"
  | "loading"
  | "playing"
  | "paused"
  | "buffering"
  | "stopping"
  | "error";

export interface PlayerState {
  session_id: number;
  phase: PlayerPhase;
  loaded: boolean;
  playing: boolean;
  buffering: boolean;
  position: number;
  duration: number;
  speed: number;
  volume: number;
  muted: boolean;
  fullscreen: boolean;
  title: string | null;
  slug: string | null;
  number: number;
  error: string | null;
  videoUrl: string | null;
}

export type PlayerCommandAction =
  | { type: "play" }
  | { type: "pause" }
  | { type: "togglePause" }
  | { type: "seek"; position: number }
  | { type: "setSpeed"; speed: number }
  | { type: "setVolume"; volume: number }
  | { type: "setMute"; muted: boolean };

export interface BufferedEpisode {
  slug: string;
  number: number;
  percent_done: number;
  segments_total: number;
  segments_done: number;
  bytes: number;
  state: string;
  error: string | null;
}

export interface BufferStatus {
  enabled: boolean;
  paused: boolean;
  pause_reasons: string[];
  cache_bytes: number;
  cache_limit_bytes: number;
  current_episode: BufferedEpisode | null;
  queue: BufferedEpisode[];
}

export interface BufferConfig {
  smart_buffer_enabled: boolean;
  buffer_episode_count: number;
  buffer_percentage: number;
  buffer_cache_limit_mb: number;
  buffer_bandwidth_limit_mbps: number;
  buffer_trigger_percent: number;
  buffer_cpu_threshold_percent: number;
}
