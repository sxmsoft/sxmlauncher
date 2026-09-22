import { useState } from "react";

import { Link2 } from "lucide-react";

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
import { formatShareCode, isCompleteShareCode } from "@/services";

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
  const [raw, setRaw] = useState("");
  const join = useJoinCode();

  const formatted = formatShareCode(raw);
  const code = raw.trim() === "" ? "" : formatted;
  const complete = isCompleteShareCode(raw);

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
          <DialogTitle>Join with a code</DialogTitle>
          <DialogDescription>
            Paste the code the host shared. The launcher opens a local bridge, so the game
            itself connects to 127.0.0.1.
          </DialogDescription>
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
            <Badge variant={complete ? "success" : "outline"}>
              {complete ? "ready to join" : "keep typing…"}
            </Badge>
            <span className="text-muted-foreground text-[11px]">
              Codes are not secret — they only point at the host's session.
            </span>
          </div>
        </div>

        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button disabled={!complete} loading={join.isPending} onClick={submit}>
            <Link2 /> Join
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
