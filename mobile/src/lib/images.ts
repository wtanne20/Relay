import type { RelayClient } from './api';
import type { HubDownload, HubRepo } from './types';

/** Image generation (stable-diffusion.cpp on the brain). */

export type EngineState = 'idle' | 'setup' | 'starting' | 'ready' | 'error';
export type GpuOwner = 'free' | 'chat' | 'images';

export interface ImageEngineStatus {
  state: EngineState;
  message: string | null;
  progress: number | null;
  external: boolean;
  port: number;
  gpu: GpuOwner;
  jobs: number;
  exe: string;
}

export interface ImagesStatus {
  engine: ImageEngineStatus;
  chat_model: 'idle' | 'starting' | 'ready' | 'error';
  models: number;
}

export interface RecipeSlot {
  id: string;
  label: string;
  required: boolean;
  categories: string[];
}

export interface Recipe {
  id: string;
  name: string;
  description: string;
  model_url: string | null;
  slots: RecipeSlot[];
  required_any: string[][];
  defaults: { width?: number; height?: number; steps?: number; cfg_scale?: number } | null;
  /** Follows edit instructions with the previous picture as reference. */
  edit: boolean;
  img2img: boolean;
  size_multiple: number;
}

export interface ImageModelFile {
  id: string;
  name: string;
  category: string;
}

export interface Suggestion {
  recipe: string;
  name: string;
  files: Record<string, string>;
}

export interface ImageCatalog {
  recipes: Recipe[];
  files: ImageModelFile[];
  suggestions: Suggestion[];
  errors: string[];
  folders: string[];
  categories: string[];
}

export interface ImageModel {
  id: string;
  name: string;
  recipe: string;
  files: Record<string, string>;
  created_at: string;
}

export interface ImageProject {
  id: string;
  name: string;
  style: string;
  negative: string;
  model_id: string | null;
  created_at: string;
  updated_at: string;
  chat_count: number;
  cover: string | null;
}

export interface ImageChat {
  id: string;
  project_id: string;
  title: string;
  model_id: string | null;
  created_at: string;
  updated_at: string;
  status: 'idle' | 'running';
  cover: string | null;
}

export interface TurnParams {
  width: number | null;
  height: number | null;
  steps: number | null;
  cfg_scale: number | null;
  strength: number | null;
  count: number;
  edit: boolean;
}

export type TurnStatus = 'queued' | 'running' | 'done' | 'error' | 'stopped';

export interface ImageTurn {
  id: string;
  chat_id: string;
  prompt: string;
  full_prompt: string;
  negative: string;
  params: TurnParams;
  base_image: string | null;
  model_id: string | null;
  model_name: string;
  status: TurnStatus;
  error: string | null;
  images: string[];
  seed: number | null;
  duration_ms: number | null;
  created_at: string;
  finished_at: string | null;
}

export interface LiveTurn {
  stage: string;
  step: number | null;
  total: number | null;
  preview: string | null;
  elapsed_ms: number;
}

export interface ImageSettings {
  sd_server_path: string;
  model_dirs: string[];
  own_folder: string;
  offload_to_cpu: boolean;
  idle_unload_minutes: number;
}

export interface GenerateRequest {
  prompt: string;
  negative?: string;
  width?: number;
  height?: number;
  steps?: number;
  cfg_scale?: number;
  strength?: number;
  count?: number;
  seed?: number;
  model_id?: string;
  base_image?: string;
  base_upload?: string;
}

const LONG = 120_000;

/** One file of a starter model. */
export interface StarterFile {
  slot: string;
  category: string;
  repo: string;
  name: string;
  size_bytes: number;
  have: boolean;
}

/** A model type with every file it needs, picked for the GPU, downloaded in one tap. */
export interface Starter {
  id: string;
  recipe: string;
  name: string;
  blurb: string;
  edit: boolean;
  /** The bigger, sharper version (the GPU has room). */
  big: boolean;
  files: StarterFile[];
  total_bytes: number;
  missing_bytes: number;
  set_up: boolean;
  downloading: boolean;
  error: string | null;
}

/** A model file in a Hugging Face repository. */
export interface ImageHubFile {
  file: string;
  name: string;
  size_bytes: number;
  quant: string | null;
  /** The folder it most likely goes in. */
  category: string;
  have: boolean;
}

export const isImageDownload = (d: HubDownload) => !!d.group?.startsWith('image');

export const imagesApi = (c: RelayClient) => ({
  status: () => c.request<ImagesStatus>('GET', '/images/status'),
  prepare: () => c.request<unknown>('POST', '/images/prepare', {}),
  unload: () => c.request<unknown>('POST', '/images/unload', {}),
  // Starting the engine (or downloading it) can take a while.
  catalog: (refresh = false) =>
    c.request<ImageCatalog>('GET', `/images/catalog${refresh ? '?refresh=1' : ''}`, undefined, LONG),
  settings: () => c.request<ImageSettings>('GET', '/images/settings'),
  patchSettings: (p: Partial<Omit<ImageSettings, 'own_folder'>>) =>
    c.request<ImageSettings>('PATCH', '/images/settings', p),

  starters: () => c.request<{ starters: Starter[]; vram_mb: number; folder: string }>('GET', '/images/downloads'),
  /** Downloads a starter's missing files; it's set up when they're in. */
  downloadStarter: (id: string) =>
    c.request<{ downloads: HubDownload[] }>('POST', `/images/downloads/${encodeURIComponent(id)}`, {}, LONG),
  hubSearch: (q: string) =>
    c.request<{ results: HubRepo[] }>('GET', `/images/hub/search?q=${encodeURIComponent(q)}`, undefined, LONG),
  hubFiles: (repo: string) =>
    c.request<{ repo: string; files: ImageHubFile[]; categories: string[] }>(
      'GET',
      `/images/hub/files?repo=${encodeURIComponent(repo)}`,
      undefined,
      LONG,
    ),
  hubDownload: (repo: string, file: string, category: string) =>
    c.request<{ download: HubDownload }>('POST', '/images/hub/download', { repo, file, category }, LONG),

  models: async () => (await c.request<{ models: ImageModel[] }>('GET', '/image-models')).models,
  createModel: (name: string, recipe: string, files: Record<string, string>) =>
    c.request<ImageModel>('POST', '/image-models', { name, recipe, files }, LONG),
  updateModel: (id: string, p: { name?: string; files?: Record<string, string> }) =>
    c.request<ImageModel>('PATCH', `/image-models/${encodeURIComponent(id)}`, p, LONG),
  deleteModel: (id: string) => c.request<unknown>('DELETE', `/image-models/${encodeURIComponent(id)}`),

  projects: async () => (await c.request<{ projects: ImageProject[] }>('GET', '/image-projects')).projects,
  project: (id: string) => c.request<ImageProject>('GET', `/image-projects/${encodeURIComponent(id)}`),
  createProject: (name: string, model_id?: string | null) =>
    c.request<ImageProject>('POST', '/image-projects', { name, model_id: model_id ?? undefined }),
  updateProject: (
    id: string,
    p: { name?: string; style?: string; negative?: string; model_id?: string },
  ) => c.request<ImageProject>('PATCH', `/image-projects/${encodeURIComponent(id)}`, p),
  deleteProject: (id: string) => c.request<unknown>('DELETE', `/image-projects/${encodeURIComponent(id)}`),

  chats: async (projectId: string) =>
    (await c.request<{ chats: ImageChat[] }>('GET', `/image-projects/${encodeURIComponent(projectId)}/chats`)).chats,
  createChat: (projectId: string) =>
    c.request<ImageChat>('POST', `/image-projects/${encodeURIComponent(projectId)}/chats`, {}),
  chat: (id: string) => c.request<ImageChat>('GET', `/image-chats/${encodeURIComponent(id)}`),
  updateChat: (id: string, p: { title?: string; model_id?: string }) =>
    c.request<ImageChat>('PATCH', `/image-chats/${encodeURIComponent(id)}`, p),
  deleteChat: (id: string) => c.request<unknown>('DELETE', `/image-chats/${encodeURIComponent(id)}`),

  turns: (chatId: string) =>
    c.request<{ turns: ImageTurn[]; live: Record<string, LiveTurn> }>(
      'GET',
      `/image-chats/${encodeURIComponent(chatId)}/turns`,
    ),
  generate: (chatId: string, req: GenerateRequest) =>
    c.request<ImageTurn>('POST', `/image-chats/${encodeURIComponent(chatId)}/generate`, req, 60_000),
  stopTurn: (id: string) => c.request<unknown>('POST', `/image-turns/${encodeURIComponent(id)}/stop`, {}),
  deleteTurn: (id: string) => c.request<unknown>('DELETE', `/image-turns/${encodeURIComponent(id)}`),
});

/** What a picture being made is doing, in words. */
export function stageLabel(live: LiveTurn | undefined, status: TurnStatus): string {
  if (status === 'queued' && (!live || live.stage === 'queued')) return 'Waiting its turn…';
  switch (live?.stage) {
    case 'waiting_for_chat':
      return 'Waiting for a chat to finish…';
    case 'starting':
      return 'Starting the image model…';
    case 'loading':
      return 'Loading the image model…';
    case 'sampling':
      return live.step != null && live.total ? `Drawing · step ${live.step} of ${live.total}` : 'Drawing…';
    case 'decoding':
      return 'Finishing…';
    case 'saving':
      return 'Saving…';
    default:
      return 'Working…';
  }
}

/** Aspect presets; sides are multiples of 64 around a megapixel. */
export const SIZES: { id: string; label: string; icon: string; w: number; h: number }[] = [
  { id: 'square', label: 'Square', icon: 'square-outline', w: 1024, h: 1024 },
  { id: 'portrait', label: 'Portrait', icon: 'crop-portrait', w: 832, h: 1216 },
  { id: 'landscape', label: 'Landscape', icon: 'crop-landscape', w: 1216, h: 832 },
  { id: 'wide', label: 'Wide', icon: 'panorama-variant-outline', w: 1344, h: 768 },
];

/** Whether each required part (and one of each either-or group) has a file. */
export function setupComplete(recipe: Recipe, files: Record<string, string>): boolean {
  const has = (id: string) => !!files[id];
  return (
    recipe.slots.filter((s) => s.required).every((s) => has(s.id)) &&
    recipe.required_any.every((g) => g.some(has))
  );
}
