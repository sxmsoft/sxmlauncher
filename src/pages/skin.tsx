import { useRef, useState, type DragEvent } from "react";

import { useNavigate } from "react-router-dom";

import { CheckCircle2, ExternalLink, LogOut, Palette, RefreshCw, Settings, Shrink, Upload, UserPlus } from "lucide-react";
import { useTranslation } from "react-i18next";

import { AccountAvatar } from "@/components/account/account-avatar";
import { LoginDialog } from "@/components/account/login-dialog";
import { PageHeader } from "@/components/common/page-header";
import { SkinPreview } from "@/components/skin/skin-preview";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { EmptyState, Skeleton } from "@/components/ui/feedback";
import {
  useAccounts,
  useActiveAccount,
  useRefreshSkin,
  useSetActiveAccount,
  useSignOut,
  useUploadSkin,
} from "@/hooks/queries";
import { downscaleSkinTo64, hdSkinScale } from "@/lib/skins";
import { openExternal } from "@/lib/window";
import { useUiStore } from "@/stores/ui";
import { cn } from "@/lib/utils";
import { PROVIDER_LABEL } from "@/types/account";
import type { AccountSummary, SkinModel, SkinUploadOutcome } from "@/types/account";

/**
 * Skin page.
 *
 * The launcher does not upload skins: each provider serves its own textures
 * (Mojang for Microsoft, Ely.by for Ely.by accounts), and the game picks them up
 * from the session. What this page is for is *verifying* what friends will see —
 * render, model, cape — and switching which profile you play as.
 */
export function SkinPage() {
  const { t } = useTranslation();
  const accounts = useAccounts();
  const active = useActiveAccount();
  const setActive = useSetActiveAccount();
  const refreshSkin = useRefreshSkin();
  const uploadSkin = useUploadSkin();
  const signOut = useSignOut();
  const navigate = useNavigate();
  const [loginOpen, setLoginOpen] = useState(false);
  const [preview, setPreview] = useState<AccountSummary | null>(null);
  const [uploadModel, setUploadModel] = useState<SkinModel>("classic");
  const [uploadOutcome, setUploadOutcome] = useState<SkinUploadOutcome | null>(null);
  const fileInput = useRef<HTMLInputElement>(null);

  /** An HD skin awaiting the user's yes/no on in-browser downscaling. */
  const [hdOffer, setHdOffer] = useState<{
    file: File;
    width: number;
    height: number;
    scale: number;
  } | null>(null);

  /**
   * Drag & drop. The whole page is a drop target: the drop itself runs the
   * exact same `handleSkinFile` path as the file picker (validation, model,
   * outcome panel), so there is one flow to reason about. `dragDepth` counts
   * enter/leave pairs because dragging across child elements fires leave on
   * each of them — a boolean would flicker the highlight on every pixel.
   */
  const [dragDepth, setDragDepth] = useState(0);
  const draggingFiles = (event: DragEvent) =>
    Array.from(event.dataTransfer?.types ?? []).includes("Files");

  const onDragEnter = (event: DragEvent) => {
    if (!draggingFiles(event) || !shown || shown.provider !== "microsoft") return;
    event.preventDefault();
    setDragDepth((depth) => depth + 1);
  };
  const onDragOver = (event: DragEvent) => {
    if (!draggingFiles(event) || !shown || shown.provider !== "microsoft") return;
    event.preventDefault(); // required for the drop event to fire
    event.dataTransfer.dropEffect = "copy";
  };
  const onDragLeave = (event: DragEvent) => {
    if (!draggingFiles(event)) return;
    setDragDepth((depth) => Math.max(0, depth - 1));
  };
  const onDrop = (event: DragEvent) => {
    if (!draggingFiles(event)) return;
    event.preventDefault();
    setDragDepth(0);
    if (!shown || shown.provider !== "microsoft") return;
    const file = event.dataTransfer?.files?.[0];
    if (!file) return;
    void handleSkinFile(file);
  };

  const dropping = dragDepth > 0;

  const shown = preview ?? active.data ?? accounts.data?.[0] ?? null;

  /** Offline profiles have nothing to refresh nor to upload to. */
  const canRefreshSkin = shown != null && shown.provider !== "offline";

  /**
   * Read the picked file, sanity-check the PNG header locally (64×64 or
   * legacy 64×32), and hand the bytes to the backend, which re-validates and
   * dispatches per provider. The same checks as `skin_upload.rs`, repeated
   * here so the user gets an instant, offline error for a wrong file.
   */
  const handleSkinFile = async (file: File) => {
    setUploadOutcome(null);
    if (!shown) return;

    const header = new Uint8Array(await file.slice(0, 24).arrayBuffer());
    const isPng =
      header[0] === 0x89 && header[1] === 0x50 && header[2] === 0x4e && header[3] === 0x47;
    // Reject non-PNGs before reading dimensions: a shorter file yields an
    // empty slice and reading IHDR out of it would throw instead of
    // producing the friendly message.
    if (!isPng) {
      setUploadOutcome({
        uploaded: false,
        skin: shown.skin,
        message: "That file is not a PNG image.",
        url: null,
      });
      return;
    }
    // IHDR width/height live at bytes 16..24 of the file.
    const view = new DataView(await file.slice(16, 24).arrayBuffer());
    const width = view.byteLength >= 8 ? view.getUint32(0) : 0;
    const height = view.byteLength >= 8 ? view.getUint32(4) : 0;
    const sizeOk = (width === 64 && height === 64) || (width === 64 && height === 32);

    if (!sizeOk) {
      // A valid PNG at a clean integer multiple of the standard layout is an
      // "HD skin": Mojang still rejects it, but it can be shrunk to its
      // standard equivalent in-browser, so offer that instead of a dead end.
      // Odd sizes (screenshots, photos, corrupt headers) get the plain
      // rejection — squashing those to 64×64 would produce garbage.
      const scale = hdSkinScale(width, height);
      if (scale != null) {
        setUploadOutcome(null);
        setHdOffer({ file, width, height, scale });
        return;
      }
      setUploadOutcome({
        uploaded: false,
        skin: shown.skin,
        message: `Skins are 64×64 pixels (or legacy 64×32) — this one is ${width}×${height}.`,
        url: null,
      });
      return;
    }

    const png = new Uint8Array(await file.arrayBuffer());
    if (png.byteLength > 256 * 1024) {
      setUploadOutcome({
        uploaded: false,
        skin: shown.skin,
        message: "That file is over the 256 KiB skin size limit.",
        url: null,
      });
      return;
    }

    uploadSkin.mutate(
      { id: shown.id, model: uploadModel, png },
      { onSuccess: setUploadOutcome },
    );
  };

  /** Downscale the offered HD skin and run it through the normal flow. */
  const acceptHdDownscale = async () => {
    if (!hdOffer || !shown) return;
    const { file, scale, height } = hdOffer;
    setHdOffer(null);
    const shrunk = await downscaleSkinTo64(file);
    if (!shrunk) {
      setUploadOutcome({
        uploaded: false,
        skin: shown.skin,
        message: "The downscale failed — export the PNG at 64×64 from your editor instead.",
        url: null,
      });
      return;
    }
    // Re-validate from scratch: the shrunk file is a new PNG with new
    // dimensions, and the user sees the same checks as a picked file.
    const standard = new File(
      [shrunk],
      `${file.name.replace(/\.png$/i, "")}-64x${height / scale}.png`,
      { type: "image/png" },
    );
    await handleSkinFile(standard);
  };

  /** Declining puts back the plain rejection the offer replaced. */
  const dismissHdOffer = () => {
    if (!hdOffer || !shown) {
      setHdOffer(null);
      return;
    }
    const { width, height } = hdOffer;
    setHdOffer(null);
    setUploadOutcome({
      uploaded: false,
      skin: shown.skin,
      message: `Skins are 64×64 pixels (or legacy 64×32) — this one is ${width}×${height}.`,
      url: null,
    });
  };

  const canUpload = shown?.provider === "microsoft";
  const uploadHint =
    shown == null
      ? null
      : shown.provider === "microsoft"
        ? t("profile.uploadMicrosoft")
        : shown.provider === "ely_by"
          ? t("profile.uploadEly")
          : t("profile.uploadOffline");

  return (
    <div
      className="flex flex-col gap-5"
      onDragEnter={onDragEnter}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
    >
      {dropping ? (
        <div
          role="status"
          className="flex items-center justify-center gap-2 rounded-xl border-2 border-dashed border-[color-mix(in_oklab,var(--primary)_55%,transparent)] bg-[color-mix(in_oklab,var(--primary)_10%,transparent)] px-4 py-2 text-xs font-medium text-[var(--foreground)]"
        >
          <Upload className="size-4" />
          Drop the PNG to apply as the {uploadModel} skin for {shown?.username ?? "…"}
        </div>
      ) : null}
      <PageHeader
        title={t("profile.title")}
        description={t("profile.subtitle")}
        actions={
          <div className="flex items-center gap-2">
            {canRefreshSkin ? (
              <Button
                size="sm"
                variant="secondary"
                className="rounded-full"
                loading={refreshSkin.isPending}
                onClick={() => {
                  if (shown) refreshSkin.mutate(shown.id);
                }}
              >
                <RefreshCw className="size-4" /> {t("profile.refresh")}
              </Button>
            ) : null}
            <Button size="sm" className="rounded-full" onClick={() => setLoginOpen(true)}>
              <UserPlus className="size-4" /> {t("profile.add")}
            </Button>
          </div>
        }
      />

      {accounts.isLoading ? (
        <div className="grid gap-5 lg:grid-cols-[minmax(0,1fr)_minmax(280px,0.6fr)]">
          <Skeleton className="h-96 w-full" />
          <Skeleton className="h-96 w-full" />
        </div>
      ) : !shown ? (
        <EmptyState
          icon={<Palette />}
          title={t("profile.emptyTitle")}
          description={t("profile.emptyBody")}
          action={
            <Button onClick={() => setLoginOpen(true)}>
              <UserPlus /> {t("profile.addAction")}
            </Button>
          }
        />
      ) : (
        <div className="grid items-start gap-5 lg:grid-cols-[320px_minmax(0,1fr)]">
          <div className="flex flex-col gap-5">
            <div
              className={cn(
                "rounded-[28px] border border-white/10 bg-black/25 p-2 shadow-[inset_0_1px_0_rgba(255,255,255,0.06)] transition-shadow",
                dropping &&
                  "ring-2 ring-[color-mix(in_oklab,var(--accent)_55%,transparent)] ring-offset-2 ring-offset-[var(--background)]",
              )}
            >
              <SkinPreview account={shown} />
            </div>

            <Card>
              <CardHeader>
                <CardTitle className="flex items-center gap-2">
                  <Upload className="size-4" /> {t("profile.uploadTitle")}
                </CardTitle>
              </CardHeader>
              <CardContent className="flex flex-col gap-3">
                <p className="text-muted-foreground text-xs leading-relaxed">{uploadHint}</p>

                {canUpload ? (
                  <p className="text-muted-foreground/70 text-[11px] leading-relaxed">
                    {t("profile.uploadDrop")}
                  </p>
                ) : null}

                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-muted-foreground text-xs font-medium">Arm model</span>
                  <div
                    role="radiogroup"
                    aria-label="Arm model"
                    className="flex overflow-hidden rounded-lg border border-white/10"
                  >
                    {(["classic", "slim"] as const).map((model) => (
                      <button
                        key={model}
                        type="button"
                        role="radio"
                        aria-checked={uploadModel === model}
                        disabled={!canUpload}
                        onClick={() => setUploadModel(model)}
                        className={
                          "px-3 py-1.5 text-xs font-medium capitalize transition-colors " +
                          (uploadModel === model
                            ? "bg-[color-mix(in_oklab,var(--primary)_18%,transparent)] text-[var(--foreground)]"
                            : "text-[var(--muted-foreground)] hover:bg-white/6")
                        }
                      >
                        {model}
                      </button>
                    ))}
                  </div>

                  <div className="flex-1" />

                  <Button
                    size="sm"
                    disabled={!canUpload}
                    loading={uploadSkin.isPending}
                    onClick={() => fileInput.current?.click()}
                  >
                    <Upload className="size-4" /> {t("profile.choosePng")}
                  </Button>
                </div>

                {/* Hidden picker; the button above opens it. The onChange key
                    resets after every pick so selecting the same file twice
                    still fires. */}
                <input
                  key={String(uploadSkin.isPending)}
                  ref={fileInput}
                  type="file"
                  accept="image/png,.png"
                  className="hidden"
                  onChange={(event) => {
                    const file = event.target.files?.[0];
                    event.target.value = "";
                    if (file) void handleSkinFile(file);
                  }}
                />

                {hdOffer ? (
                  <div
                    role="status"
                    className="flex flex-col gap-2 rounded-lg border border-[color-mix(in_oklab,var(--warning)_45%,transparent)] bg-[color-mix(in_oklab,var(--warning)_10%,transparent)] p-3 text-xs"
                  >
                    <span className="font-medium">
                      HD skin detected — {hdOffer.width}×{hdOffer.height}, {hdOffer.scale}× the
                      standard layout
                    </span>
                    <span className="text-muted-foreground leading-relaxed">
                      Mojang rejects HD skins: the official upload endpoint only accepts 64×64 (or
                      legacy 64×32). Shrink it here to its standard equivalent — pixels are
                      averaged, so a clean upscale comes back exactly like the original.
                    </span>
                    <div className="flex flex-wrap items-center gap-2">
                      <Button size="sm" loading={uploadSkin.isPending} onClick={() => void acceptHdDownscale()}>
                        <Shrink className="size-3.5" /> Downscale to 64×{hdOffer.height / hdOffer.scale} &amp;
                        upload
                      </Button>
                      <Button size="sm" variant="ghost" onClick={dismissHdOffer}>
                        Keep the original
                      </Button>
                    </div>
                  </div>
                ) : null}

                {uploadOutcome ? (
                  <div
                    role="status"
                    className="flex flex-col gap-1.5 rounded-lg border border-white/10 bg-black/20 p-3 text-xs"
                  >
                    {uploadOutcome.uploaded ? (
                      <span className="flex items-center gap-1.5 font-medium text-[var(--success)]">
                        <CheckCircle2 className="size-3.5" /> Skin uploaded to Mojang
                      </span>
                    ) : (
                      <span className="font-medium">{uploadOutcome.message}</span>
                    )}
                    {uploadOutcome.url ? (
                      <Button
                        size="sm"
                        variant="secondary"
                        className="self-start"
                        onClick={() => void openExternal(uploadOutcome.url!)}
                      >
                        <ExternalLink className="size-3.5" /> Continue on ely.by
                      </Button>
                    ) : null}
                  </div>
                ) : null}
              </CardContent>
            </Card>
          </div>

          <div className="flex flex-col gap-4">
            <Card className="flex flex-row items-center gap-3.5 p-4">
              <div className="grid size-10 shrink-0 place-items-center rounded-[10px] border border-[var(--border)] bg-[#2f2f2f] text-[11px] font-bold text-[#00a4ef]">
                {shown.provider === "microsoft" ? "MS" : shown.provider === "ely_by" ? "EL" : "OFF"}
              </div>
              <div className="min-w-0 flex-1">
                <div className="text-sm font-semibold">
                  {shown.provider === "microsoft" ? t("profile.connected") : PROVIDER_LABEL[shown.provider]}
                </div>
                <div className="mt-0.5 truncate text-xs text-[var(--text-muted)]">{shown.username}</div>
              </div>
              {shown.id === active.data?.id ? <Badge variant="success">{t("profile.active")}</Badge> : <Badge variant="outline">{t("profile.previewing")}</Badge>}
            </Card>

            <div className="grid grid-cols-2 gap-2.5">
              <button
                type="button"
                className="rounded-[var(--radius-md)] border border-[var(--border)] bg-[var(--surface-2)] p-4 text-left hover:border-[var(--border-strong)] hover:bg-[var(--accent-dim)]"
                onClick={() => {
                  useUiStore.getState().openSettings("appearance");
                  void navigate("/settings");
                }}
              >
                <Settings className="mb-2 size-3.5 text-[var(--accent-soft)]" />
                <div className="text-[13px] font-semibold">Appearance</div>
                <div className="text-[11px] text-[var(--text-faint)]">Theme and wallpaper</div>
              </button>
              <button
                type="button"
                className="rounded-[var(--radius-md)] border border-[var(--border)] bg-[var(--surface-2)] p-4 text-left hover:border-[var(--border-strong)] hover:bg-[var(--accent-dim)]"
                onClick={() => setLoginOpen(true)}
              >
                <UserPlus className="mb-2 size-3.5 text-[var(--accent-soft)]" />
                <div className="text-[13px] font-semibold">Add account</div>
                <div className="text-[11px] text-[var(--text-faint)]">Microsoft or Ely.by</div>
              </button>
              <button
                type="button"
                className="rounded-[var(--radius-md)] border border-[var(--border)] bg-[var(--surface-2)] p-4 text-left hover:border-[var(--border-strong)] hover:bg-[var(--accent-dim)]"
                onClick={() => signOut.mutate(shown.id)}
                disabled={signOut.isPending}
              >
                <LogOut className="mb-2 size-3.5 text-[var(--accent-soft)]" />
                <div className="text-[13px] font-semibold">Sign out</div>
                <div className="text-[11px] text-[var(--text-faint)]">{PROVIDER_LABEL[shown.provider]}</div>
              </button>
            </div>

          <Card>
            <CardHeader>
              <CardTitle>Profiles</CardTitle>
            </CardHeader>
            <CardContent className="flex flex-col gap-2">
              {(accounts.data ?? []).map((account) => {
                const isActive = account.id === active.data?.id;
                const isPreviewed = account.id === shown.id;
                return (
                  // A real <button> here would nest the "Use" <button> inside
                  // it, which is invalid HTML and breaks keyboard activation, so
                  // the row is a div that re-implements the button semantics.
                  <div
                    key={account.id}
                    role="button"
                    tabIndex={0}
                    aria-pressed={isPreviewed}
                    aria-label={`Preview ${account.username}`}
                    onClick={() => setPreview(account)}
                    onKeyDown={(event) => {
                      if (event.key === "Enter" || event.key === " ") {
                        event.preventDefault();
                        setPreview(account);
                      }
                    }}
                    className={
                      "flex cursor-pointer items-center gap-3 rounded-xl border p-3 text-left transition-colors " +
                      "focus-visible:border-[color-mix(in_oklab,var(--primary)_45%,transparent)] focus-visible:outline-none " +
                      (isPreviewed
                        ? "border-[color-mix(in_oklab,var(--primary)_45%,transparent)] bg-[color-mix(in_oklab,var(--primary)_10%,transparent)]"
                        : "border-white/8 bg-black/20 hover:bg-white/6")
                    }
                  >
                    <AccountAvatar account={account} size={36} />
                    <span className="flex min-w-0 flex-1 flex-col">
                      <span className="truncate text-sm font-medium">{account.username}</span>
                      <span className="text-muted-foreground text-[11px]">
                        {PROVIDER_LABEL[account.provider]} · {account.skin.model}
                      </span>
                    </span>
                    {isActive ? (
                      <Badge variant="success">active</Badge>
                    ) : (
                      <Button
                        size="sm"
                        variant="secondary"
                        onClick={(event) => {
                          event.stopPropagation();
                          setActive.mutate(account.id);
                        }}
                        loading={setActive.isPending}
                      >
                        Use
                      </Button>
                    )}
                  </div>
                );
              })}

              <p className="text-muted-foreground mt-2 text-xs leading-relaxed">
                Offline profiles always render the fallback avatar: without a signed-in
                provider there is no skin texture to fetch.
              </p>
            </CardContent>
          </Card>
          </div>
        </div>
      )}

      <LoginDialog open={loginOpen} onOpenChange={setLoginOpen} />
    </div>
  );
}
