import { useCallback, useEffect, useReducer, useRef } from "react";
import type { ModelService } from "../types";

export interface ModelCatalog {
  models?: string[];
  loading: boolean;
  error?: string;
}

interface Entry extends ModelCatalog {
  connection: ModelService;
}

export function automaticCatalog(service: ModelService) {
  return service.provider === "openai" || service.provider === "volc_ark";
}

function sameConnection(a: ModelService, b: ModelService) {
  return a.id === b.id && a.provider === b.provider && a.endpoint === b.endpoint && a.api_key === b.api_key;
}

export function useModelCatalogs(saved: ModelService[], fetchModels: (id: string) => Promise<string[]>) {
  const entries = useRef(new Map<string, Entry>());
  const mounted = useRef(false);
  const [, redraw] = useReducer((revision: number) => revision + 1, 0);

  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  const load = useCallback((service: ModelService, force: boolean) => {
    if (!automaticCatalog(service) || !service.endpoint.trim() || !service.api_key.trim()) return;
    const previous = entries.current.get(service.id);
    const matches = previous && sameConnection(previous.connection, service);
    if (matches && (previous.loading || !force)) return;
    const entry: Entry = { connection: service, models: matches ? previous.models : undefined, loading: true };
    entries.current.set(service.id, entry);
    redraw();
    void fetchModels(service.id).then((models) => { entry.models = models; })
      .catch((error) => { entry.error = String(error); })
      .finally(() => {
        entry.loading = false;
        if (mounted.current && entries.current.get(service.id) === entry) redraw();
      });
  }, [fetchModels]);

  useEffect(() => {
    for (const [id, entry] of entries.current) {
      if (!saved.some((service) => sameConnection(service, entry.connection))) entries.current.delete(id);
    }
    for (const service of saved) load(service, false);
  }, [saved, load]);

  function canFetch(service: ModelService) {
    return automaticCatalog(service) && Boolean(service.api_key.trim() && service.endpoint.trim())
      && saved.some((item) => sameConnection(item, service));
  }

  function catalog(service: ModelService): ModelCatalog | undefined {
    const entry = entries.current.get(service.id);
    return entry && canFetch(service) && sameConnection(entry.connection, service) ? entry : undefined;
  }

  function refresh(service: ModelService) {
    if (canFetch(service)) load(service, true);
  }
  return { catalog, canFetch, refresh };
}
