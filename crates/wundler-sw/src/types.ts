export interface ManifestRequest {
  entry_point: string;
  cached_hashes: string[];
  build_id?: string;
}

export interface ManifestResponse {
  build_id: string;
  fetch_urls: string[];
  prefetch_urls: string[];
  ttl: number;
}

export interface StaticManifest {
  build_id: string;
  chunks: Array<{ id: string; hash: string; modules: string[] }>;
  entry_chunks: Record<string, string[]>;
  module_index: Record<string, string>;
}

export interface CacheLike {
  put(request: RequestInfo | URL, response: Response): Promise<void>;
  match(request: RequestInfo | URL): Promise<Response | undefined>;
  keys(): Promise<ReadonlyArray<Request>>;
  delete(request: RequestInfo | URL): Promise<boolean>;
}

export interface SwConfig {
  cdnBaseUrl: string;
  absBaseUrl: string;
  cache?: CacheLike;
}
