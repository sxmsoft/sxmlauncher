/**
 * Bridge between the backend's event streams and the client stores.
 *
 * Mounted exactly once (in `AppShell`). Three subscriptions:
 *   * `job://progress` → job store (every progress bar in the app)
 *   * `session://event` → session store (guests, kicks, relay fallback)
 *   * `game://state`   → running-game set, so the Play button flips to Stop
 *                        without the UI ever polling
 *
 * It also keeps hosted worlds fresh: Redis-listed sessions are described by
 * heartbeats, and the guest list only lives in the backend, so a slow interval
 * re-reads `host_status` for each session this machine owns.
 */

import { useEffect } from "react";

import { useQueryClient } from "@tanstack/react-query";

import { qk } from "@/lib/query-client";
import { networkService, onGameState, onProgress, onSession } from "@/services";
import { useJobsStore } from "@/stores/jobs";
import { useSessionsStore } from "@/stores/sessions";
import { toast } from "@/stores/ui";
import type { GameState } from "@/types/instance";
import { useRunningInstances } from "./queries";

/** How often to re-read guest lists for worlds we are hosting. */
const HOST_REFRESH_MS = 15_000;

export function useBackendEvents(): void {
  const client = useQueryClient();
  const ingestJob = useJobsStore((state) => state.ingest);
  const ingestSession = useSessionsStore((state) => state.ingest);
  const patchHost = useSessionsStore((state) => state.patchHost);
  const removeHost = useSessionsStore((state) => state.removeHost);

  // Seed from the backend: a game may already be running when the UI opens.
  useRunningInstances();

  useEffect(() => {
    const unlisteners: Array<() => void> = [];
    let disposed = false;

    const track = (promise: Promise<() => void>) => {
      void promise.then((unlisten) => {
        if (disposed) unlisten();
        else unlisteners.push(unlisten);
      });
    };

    track(onProgress(ingestJob));

    track(
      onSession((event) => {
        ingestSession(event);
        if (event.kind === "relay_fallback" && event.message) {
          toast.warning("Relay fallback", event.message);
        }
        if (event.kind === "host_closed") {
          removeHost(event.id);
        }
      }),
    );

    track(
      onGameState((event) => {
        void client.invalidateQueries({ queryKey: qk.running });
        void client.invalidateQueries({ queryKey: qk.instances });

        const wording: Record<GameState, string> = {
          starting: "Starting the game",
          running: "Game running",
          exited: "Game closed",
          crashed: "The game crashed",
        };
        const detail = event.message ?? undefined;
        if (event.state === "crashed") toast.error(detail ?? wording.crashed, "Game crashed");
        else if (event.state === "exited") toast.info(wording.exited, detail);
      }),
    );

    return () => {
      disposed = true;
      for (const unlisten of unlisteners) unlisten();
    };
  }, [client, ingestJob, ingestSession, patchHost, removeHost]);

  // Hosted worlds: refresh guest lists and player counts on an interval.
  useEffect(() => {
    let timer: number | undefined;

    const refresh = async () => {
      const hosts = Object.keys(useSessionsStore.getState().hosts);
      for (const id of hosts) {
        try {
          const status = await networkService.hostStatus(id);
          patchHost(status);
        } catch {
          // The session ended underneath us (or the directory dropped); the
          // next successful refresh or the user's Stop button will settle it.
        }
      }
    };

    timer = window.setInterval(() => void refresh(), HOST_REFRESH_MS);
    void refresh();
    return () => window.clearInterval(timer);
  }, [patchHost]);
}
