import { useEffect, useRef, useState } from "react";
import type { LearningApi } from "../api";
import type { LearningActivity } from "../../types";

export function useLearningActivityController(api: LearningApi, onOpen: () => void) {
  const [active, setActive] = useState<LearningActivity[]>([]);
  const [error, setError] = useState<string | null>(null);
  const onOpenRef = useRef(onOpen);
  onOpenRef.current = onOpen;

  useEffect(() => {
    let stopped = false;
    let generation = 0;
    const unlisteners: Array<() => void> = [];

    async function refresh() {
      const request = ++generation;
      try {
        const activity = await api.loadActivity(1, 0);
        if (stopped || request !== generation) return;
        setActive(activity.active);
        setError(null);
      } catch (failure) {
        if (!stopped && request === generation) setError(String(failure));
      }
    }

    async function openRequested() {
      try {
        const requested = await api.takeOpenRequest();
        if (!stopped && requested) onOpenRef.current();
      } catch (failure) {
        if (!stopped) setError(String(failure));
      }
    }

    function keepListener(stop: () => void) {
      if (stopped) stop();
      else unlisteners.push(stop);
    }

    void api.listenUpdated(() => { void refresh(); })
      .then(keepListener)
      .catch((failure) => { if (!stopped) setError(String(failure)); })
      .finally(() => { if (!stopped) void refresh(); });
    void api.listenOpenRequested(() => { void openRequested(); })
      .then(keepListener)
      .catch((failure) => { if (!stopped) setError(String(failure)); })
      .finally(() => { if (!stopped) void openRequested(); });

    return () => {
      stopped = true;
      generation += 1;
      unlisteners.forEach((stop) => stop());
    };
  }, [api]);

  return { active, error };
}
