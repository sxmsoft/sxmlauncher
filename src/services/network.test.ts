import { describe, expect, it } from "vitest";

import { formatShareCode, isCompleteShareCode } from "./network";

/** Host-style relay code: 34 base32 chars, the 21-byte ConnectCode payload. */
const HOST_CODE = "SXM1-AEAI-RMQG-NTKC-WSQT-ELAX-3T6N-CYAB-FF6E-NQ";

describe("formatShareCode", () => {
  it("keeps a full host code instead of cutting it to 16 characters", () => {
    expect(formatShareCode(HOST_CODE)).toBe(HOST_CODE);
    const payload = HOST_CODE.replace(/-/g, "").slice(4);
    expect(payload).toHaveLength(34);
    expect(formatShareCode(HOST_CODE).replace(/-/g, "").slice(4)).toBe(payload);
  });

  it("rebuilds grouping from a sloppy paste", () => {
    const sloppy = `  ${HOST_CODE.toLowerCase().replaceAll("-", " ")}  `;
    expect(formatShareCode(sloppy)).toBe(HOST_CODE);
  });

  it("stops after the 34th payload character", () => {
    expect(formatShareCode(`${HOST_CODE}-ZZZZ`)).toBe(HOST_CODE);
  });

  it("drops a repeated prefix so a paste onto SXM1 stays complete", () => {
    expect(formatShareCode(`SXM1${HOST_CODE}`)).toBe(HOST_CODE);
    expect(formatShareCode(`SXM1-${HOST_CODE}`)).toBe(HOST_CODE);
    expect(isCompleteShareCode(`SXM1${HOST_CODE}`)).toBe(true);
  });
});

describe("isCompleteShareCode", () => {
  it("accepts a full 21-byte code", () => {
    expect(isCompleteShareCode(HOST_CODE)).toBe(true);
    expect(isCompleteShareCode(HOST_CODE.toLowerCase().replaceAll("-", ""))).toBe(true);
  });

  it("rejects the old 4-group form that decodes to 10 bytes", () => {
    expect(isCompleteShareCode("SXM1-7K4Q-2M9V-TR3N-XW8P")).toBe(false);
    expect(isCompleteShareCode(HOST_CODE.slice(0, "SXM1-XXXX-XXXX-XXXX-XXXX".length))).toBe(false);
  });

  it("rejects a payload that is one character short", () => {
    const compact = HOST_CODE.replace(/-/g, "");
    expect(isCompleteShareCode(compact.slice(0, -1))).toBe(false);
  });
});
