/**
 * Backend join/host failures carry a stable token (`INVITE_EXPIRED: …`) so the
 * UI can show Turkish or English without parsing English sentences.
 */

import type { TFunction } from "i18next";

const TOKENS = [
  "INVITE_EXPIRED",
  "INVITE_LISTING_GONE",
  "INVITE_MALFORMED",
  "DIRECTORY_UNREACHABLE",
  "STUN_UNREACHABLE",
  "JOIN_UNREACHABLE",
] as const;

export type InviteToken = (typeof TOKENS)[number];

export function inviteToken(message: string): InviteToken | null {
  for (const token of TOKENS) {
    if (message.includes(token)) return token;
  }
  return null;
}

export function translateInviteMessage(message: string, t: TFunction): string {
  const token = inviteToken(message);
  if (!token) return message;
  return t(`invite.errors.${token}`);
}
