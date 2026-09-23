import { useState } from "react";

import { Link2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Textarea } from "@/components/ui/input";
import { useJoinCode } from "@/hooks/queries";
import { translateInviteMessage } from "@/lib/invite-errors";
import { formatShareCode, invalidShareCodeChars, isCompleteShareCode } from "@/services";

/**
 * Join with a share code.
 *
 * The code is normalized and validated *before* the network round trip, so a
 * typo fails instantly with a clear message instead of a timeout. The dialog
 * shows the parsing state so the user can see the code being accepted.
 */
export function JoinCodeDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { t } = useTranslation();
  const [raw, setRaw] = useState("");
  const join = useJoinCode();
  const failure = join.error
    ? translateInviteMessage(join.error instanceof Error ? join.error.message : String(join.error), t)
    : null;

  const formatted = formatShareCode(raw);
  const code = raw.trim() === "" ? "" : formatted;
  const invalidChars = invalidShareCodeChars(raw);
  const complete = invalidChars.length === 0 && isCompleteShareCode(raw);

  const submit = () => {
    join.mutate(code, {
      onSuccess: () => {
        setRaw("");
        onOpenChange(false);
      },
    });
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="w-[min(94vw,40rem)]">
        <DialogHeader>
          <DialogTitle>{t("invite.title")}</DialogTitle>
          <DialogDescription>{t("invite.description")}</DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-3">
          <Textarea
            value={code}
            onChange={(event) => setRaw(event.target.value)}
            placeholder="SXM1-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XX"
            rows={2}
            className="min-h-16 resize-none text-center font-mono text-sm leading-6 break-all tracking-normal"
            autoFocus
            onKeyDown={(event) => {
              if (event.key === "Enter" && complete) {
                event.preventDefault();
                submit();
              }
            }}
          />
          <div className="flex items-center gap-2">
            <Badge variant={complete ? "success" : invalidChars.length > 0 ? "destructive" : "outline"}>
              {complete
                ? t("invite.ready")
                : invalidChars.length > 0
                  ? t("invite.invalid")
                  : t("invite.typing")}
            </Badge>
            <span className="text-muted-foreground text-[11px]">{t("invite.hint")}</span>
          </div>
          {invalidChars.length > 0 ? (
            <p className="text-[var(--destructive)] text-xs leading-relaxed">
              {t("invite.invalidChars", { chars: invalidChars.join(" ") })}
            </p>
          ) : null}
          {failure ? <p className="text-[var(--destructive)] text-xs leading-relaxed">{failure}</p> : null}
        </div>

        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            {t("invite.cancel")}
          </Button>
          <Button disabled={!complete} loading={join.isPending} onClick={submit}>
            <Link2 /> {t("invite.submit")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
