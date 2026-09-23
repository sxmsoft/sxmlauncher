import { useEffect, useMemo, useRef } from "react";

import { useLocation } from "react-router-dom";
import { useTranslation } from "react-i18next";

import { useInstances, useRunningInstances, useSettings } from "@/hooks/queries";
import { buildPresence, isPresenceWork, type PresenceCopy } from "@/lib/discord-presence";
import { discordService } from "@/services/discord";
import { useJobsStore } from "@/stores/jobs";
import { useSessionsStore } from "@/stores/sessions";

/**
 * Push the current page, download, host, or running game to Discord.
 *
 * Mounted once in the shell. When a game process ends, the running list drops
 * and this sends the page status again. Closing the launcher clears the
 * activity in Rust (`CloseRequested` and process shutdown), including when
 * Discord is not running — that path is a no-op.
 */
export function useDiscordPresence(): void {
  const location = useLocation();
  const { t, i18n } = useTranslation();
  const instances = useInstances();
  const running = useRunningInstances();
  const settings = useSettings();
  const jobs = useJobsStore((state) => state.jobs);
  const hosts = useSessionsStore((state) => state.hosts);
  const hostForInstance = useSessionsStore((state) => state.hostForInstance);
  const guests = useSessionsStore((state) => state.guests);
  const sinceRef = useRef<Record<string, number>>({});

  const copy = useMemo<PresenceCopy>(
    () => ({
      home: t("presence.home"),
      library: t("presence.library"),
      modpacks: t("presence.modpacks"),
      profile: t("presence.profile"),
      activity: t("presence.activity"),
      settings: t("presence.settings"),
      multiplayer: t("presence.multiplayer"),
      download: t("presence.download"),
      launching: t("presence.launching"),
      hosting: t("presence.hosting"),
      joining: t("presence.joining"),
      hostingShort: t("presence.hostingShort"),
      multiplayerShort: t("presence.multiplayerShort"),
      brand: t("presence.brand"),
    }),
    [t, i18n.language],
  );

  const runningIds = useMemo(
    () => (running.data ?? []).map((entry) => entry.instanceId),
    [running.data],
  );

  const playingSinceMs = useMemo(() => {
    const live = new Set(runningIds);
    for (const id of Object.keys(sinceRef.current)) {
      if (!live.has(id)) delete sinceRef.current[id];
    }
    for (const id of runningIds) {
      if (!sinceRef.current[id]) sinceRef.current[id] = Date.now();
    }
    const hosted = runningIds.find((id) => hostForInstance[id]);
    const id = hosted ?? runningIds[0];
    return id ? (sinceRef.current[id] ?? null) : null;
  }, [runningIds, hostForInstance]);

  const payload = useMemo(() => {
    const hostEntry = Object.values(hosts)[0];
    const guestEntry = Object.values(guests)[0];
    const hostInstanceId =
      Object.entries(hostForInstance).find(([, sessionId]) => hosts[sessionId])?.[0] ??
      Object.keys(hostForInstance)[0] ??
      null;
    const active =
      jobs.find((job) =>
        isPresenceWork({
          kind: job.kind,
          stage: job.stage,
          label: job.label,
          finished: job.finished,
          error: job.error,
          startedAtMs: job.startedAtMs,
        }),
      ) ?? null;

    return buildPresence({
      pathname: location.pathname,
      copy,
      instances: (instances.data ?? []).map((instance) => ({
        id: instance.id,
        name: instance.name,
        gameVersion: instance.gameVersion,
        loader: instance.loader.kind,
      })),
      runningIds,
      playingSinceMs,
      hostName: hostEntry?.summary.name ?? null,
      hostInstanceId,
      joinName: guestEntry?.serverName ?? null,
      work: active
        ? {
            kind: active.kind,
            stage: active.stage,
            label: active.label || active.currentItem || "",
            finished: active.finished,
            error: active.error,
            startedAtMs: active.startedAtMs || null,
          }
        : null,
    });
  }, [
    location.pathname,
    copy,
    instances.data,
    runningIds,
    playingSinceMs,
    hosts,
    hostForInstance,
    guests,
    jobs,
  ]);

  const applicationId = settings.data?.discordApplicationId ?? "";
  const sent = useRef("");

  useEffect(() => {
    const key = `${applicationId}\n${JSON.stringify(payload)}`;
    const timer = window.setTimeout(() => {
      if (sent.current === key) return;
      sent.current = key;
      void discordService.set(payload).catch(() => {
        // Browser preview and a closed Discord client both land here or no-op.
      });
    }, 300);
    return () => window.clearTimeout(timer);
  }, [payload, applicationId]);
}
