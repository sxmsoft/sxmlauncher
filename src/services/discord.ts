/**
 * Discord Rich Presence.
 *
 * The backend no-ops when Discord is closed or no application id is set, so
 * callers can fire these without treating a missing client as an error.
 */

import { call } from "./ipc";

export interface PresenceActivity {
  details: string;
  state: string;
  startUnixMs: number | null;
}

export interface PresenceStatus {
  /** `true` when an application id is configured. Discord may still be closed. */
  enabled: boolean;
}

export const discordService = {
  set: (activity: PresenceActivity) => call<PresenceStatus>("discord_presence_set", { activity }),

  clear: () => call<void>("discord_presence_clear"),
};
