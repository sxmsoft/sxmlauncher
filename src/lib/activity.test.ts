import { describe, expect, it } from "vitest";

import { activityPresentation } from "./activity";
import type { ProgressEvent } from "@/types/modpack";

function job(partial: Partial<ProgressEvent> & Pick<ProgressEvent, "label" | "stage" | "kind">): ProgressEvent {
  return {
    jobId: "job",
    completedUnits: 0,
    totalUnits: 0,
    bytesPerSecond: 0,
    currentItem: null,
    detail: null,
    finished: false,
    error: null,
    startedAtMs: 0,
    ...partial,
  };
}

describe("activityPresentation", () => {
  it("treats a live host as a calm running service", () => {
    const view = activityPresentation(
      job({ kind: "p2p_host", stage: "resolving", label: "Hosting Survival" }),
    );
    expect(view.mode).toBe("steady");
    expect(view.status).toBe("Service Running");
    expect(view.showProgress).toBe(false);
  });

  it("keeps a bar while files are actually downloading", () => {
    const view = activityPresentation(
      job({
        kind: "modpack_install",
        stage: "downloading",
        label: "Fabulously Optimized",
        completedUnits: 40,
        totalUnits: 100,
      }),
    );
    expect(view.mode).toBe("work");
    expect(view.status).toBe("Downloading");
    expect(view.showProgress).toBe(true);
    expect(view.progress).toBe(40);
  });

  it("shows Starting while the world port is still opening", () => {
    const view = activityPresentation(
      job({
        kind: "p2p_host",
        stage: "extracting",
        label: "Waiting for Minecraft server",
        detail: "127.0.0.1:25565",
      }),
    );
    expect(view.mode).toBe("work");
    expect(view.status).toBe("Starting…");
    expect(view.showProgress).toBe(true);
    expect(view.progress).toBeNull();
  });

  it("shows Resolving files for a real resolve, not a host snapshot", () => {
    const view = activityPresentation(
      job({ kind: "instance_install", stage: "resolving", label: "Resolving Fabric" }),
    );
    expect(view.mode).toBe("work");
    expect(view.status).toBe("Resolving files");
    expect(view.showProgress).toBe(true);
  });

  it("renders NAT discovery failure without a progress bar", () => {
    const view = activityPresentation(
      job({
        kind: "p2p_host",
        stage: "registering",
        label: "NAT discovery failed",
        detail: "STUN servers are unreachable",
      }),
    );
    expect(view.mode).toBe("error");
    expect(view.status).toBe("STUN servers are unreachable");
    expect(view.showProgress).toBe(false);
  });

  it("renders an unreachable STUN label as an error even while the job is active", () => {
    const view = activityPresentation(
      job({
        kind: "p2p_host",
        stage: "connecting_p2p",
        label: "STUN servers are unreachable",
      }),
    );
    expect(view.mode).toBe("error");
    expect(view.showProgress).toBe(false);
  });

  it("renders a failed stage as an error", () => {
    const view = activityPresentation(
      job({
        kind: "p2p_connect",
        stage: "failed",
        label: "Direct connection failed",
        error: "transport error",
        finished: true,
      }),
    );
    expect(view.mode).toBe("error");
    expect(view.status).toBe("transport error");
    expect(view.showProgress).toBe(false);
  });

  it("treats listening and a published session as steady", () => {
    expect(
      activityPresentation(
        job({
          kind: "p2p_host",
          stage: "registering",
          label: "Detected local Minecraft server",
          detail: "127.0.0.1:25565",
        }),
      ).status,
    ).toBe("Listening");

    const published = activityPresentation(
      job({ kind: "p2p_host", stage: "running", label: "Session published" }),
    );
    expect(published.mode).toBe("steady");
    expect(published.showProgress).toBe(false);
  });
});
