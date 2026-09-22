/**
 * P2P sessions as the UI sees them.
 *
 * Two maps plus an event log:
 *   * `hosts`  — worlds *this* machine is serving (share code, guests, NAT mode)
 *   * `guests` — remote worlds this machine has joined (local bridge address)
 *
 * `HostStatus` deliberately carries no instance id (the backend treats hosting
 * and instances as independent), so the mapping instance → session lives here,
 * set by whichever action started the host. That keeps the Instance page's
 * "Host World to Friends" toggle a pure lookup.
 */

import { create } from "zustand";

import type { HostStatus, JoinStatus, SessionEventPayload } from "@/types/server";

const MAX_EVENTS = 60;

interface SessionsState {
  hosts: Record<string, HostStatus>;
  guests: Record<string, JoinStatus>;
  /** instanceId → sessionId, for hosted worlds. */
  hostForInstance: Record<string, string>;
  events: SessionEventPayload[];
  setHost: (status: HostStatus, instanceId?: string | null) => void;
  patchHost: (status: HostStatus) => void;
  removeHost: (sessionId: string) => void;
  hostOfInstance: (instanceId: string | null | undefined) => HostStatus | null;
  setGuest: (status: JoinStatus) => void;
  removeGuest: (sessionId: string) => void;
  ingest: (event: SessionEventPayload) => void;
  clearEvents: () => void;
  reset: () => void;
}

export const useSessionsStore = create<SessionsState>((set, get) => ({
  hosts: {},
  guests: {},
  hostForInstance: {},
  events: [],

  setHost: (status, instanceId) =>
    set((state) => ({
      hosts: { ...state.hosts, [status.id]: status },
      hostForInstance: instanceId
        ? { ...state.hostForInstance, [instanceId]: status.id }
        : state.hostForInstance,
    })),

  patchHost: (status) =>
    set((state) => ({ hosts: { ...state.hosts, [status.id]: status } })),

  removeHost: (sessionId) =>
    set((state) => {
      const hosts = { ...state.hosts };
      delete hosts[sessionId];
      const hostForInstance = Object.fromEntries(
        Object.entries(state.hostForInstance).filter(([, id]) => id !== sessionId),
      );
      return { hosts, hostForInstance };
    }),

  hostOfInstance: (instanceId) => {
    if (!instanceId) return null;
    const sessionId = get().hostForInstance[instanceId];
    return sessionId ? (get().hosts[sessionId] ?? null) : null;
  },

  setGuest: (status) => set((state) => ({ guests: { ...state.guests, [status.id]: status } })),

  removeGuest: (sessionId) =>
    set((state) => {
      const guests = { ...state.guests };
      delete guests[sessionId];
      return { guests };
    }),

  /**
   * Session events are both a log and a trigger: `guest_joined` /
   * `guest_left` also refresh the affected host, so player counts and the guest
   * list stay correct without the UI polling `host_status`.
   */
  ingest: (event) =>
    set((state) => ({
      events: [event, ...state.events].slice(0, MAX_EVENTS),
      hosts: state.hosts[event.id]
        ? {
            ...state.hosts,
            [event.id]: {
              ...state.hosts[event.id]!,
              summary: event.players
                ? { ...state.hosts[event.id]!.summary, players: event.players }
                : state.hosts[event.id]!.summary,
              guests:
                event.kind === "guest_left"
                  ? state.hosts[event.id]!.guests.filter(
                      (guest) => guest.peerId !== event.peerId,
                    )
                  : event.kind === "guest_joined" &&
                      event.peerId &&
                      !state.hosts[event.id]!.guests.some(
                        (guest) => guest.peerId === event.peerId,
                      )
                    ? [
                        ...state.hosts[event.id]!.guests,
                        {
                          peerId: event.peerId,
                          address: "",
                          username: event.username,
                          protocolVersion: null,
                          joinedAt: new Date().toISOString(),
                        },
                      ]
                    : state.hosts[event.id]!.guests,
            },
          }
        : state.hosts,
    })),

  clearEvents: () => set({ events: [] }),

  reset: () => set({ hosts: {}, guests: {}, hostForInstance: {}, events: [] }),
}));
