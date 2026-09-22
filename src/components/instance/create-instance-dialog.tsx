import { useState } from "react";

import { Plus } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Field, Input, Select } from "@/components/ui/input";
import { NumberInput, Switch } from "@/components/ui/switch";
import { useCreateInstance, useInstallInstance, useVersions } from "@/hooks/queries";
import type { LoaderKind } from "@/types/instance";

const LOADERS: LoaderKind[] = ["vanilla", "fabric", "quilt", "forge", "neoforge"];

/**
 * Create a new isolated instance.
 *
 * Pick a vanilla release from the Mojang manifest, optionally attach a mod
 * loader, and either install right away or just create the shell. Installing
 * streams into the job tracker, so the dialog can close immediately.
 */
export function CreateInstanceDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { data: versions, isLoading } = useVersions(true);
  const create = useCreateInstance();
  const install = useInstallInstance();

  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [gameVersion, setGameVersion] = useState("1.21.1");
  const [loader, setLoader] = useState<LoaderKind>("fabric");
  const [maxMemory, setMaxMemory] = useState(6144);
  const [installNow, setInstallNow] = useState(true);

  const reset = () => {
    setName("");
    setDescription("");
    setMaxMemory(6144);
    setInstallNow(true);
  };

  const submit = () => {
    create.mutate(
      {
        name: name.trim(),
        description: description.trim() || undefined,
        gameVersion,
        loader: loader === "vanilla" ? undefined : { kind: loader, version: null, build: null },
        memory: { minMb: Math.min(2048, maxMemory), maxMb: maxMemory },
        installNow: false,
      },
      {
        onSuccess: (instance) => {
          onOpenChange(false);
          reset();
          if (installNow) install.mutate(instance.id);
        },
      },
    );
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>New instance</DialogTitle>
          <DialogDescription>
            Every instance gets its own mods, configs, resource packs and worlds — two
            setups never share a classpath.
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-4">
          <Field label="Name" htmlFor="new-instance-name">
            <Input
              id="new-instance-name"
              value={name}
              onChange={(event) => setName(event.target.value)}
              placeholder="Fabric 1.21.1"
              maxLength={64}
            />
          </Field>

          <Field label="Description" htmlFor="new-instance-desc">
            <Input
              id="new-instance-desc"
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              placeholder="Optional"
              maxLength={140}
            />
          </Field>

          <div className="grid gap-4 sm:grid-cols-2">
            <Field
              label="Minecraft version"
              htmlFor="new-instance-version"
              hint={isLoading ? "loading the Mojang manifest…" : undefined}
            >
              <Select
                id="new-instance-version"
                value={gameVersion}
                onChange={(event) => setGameVersion(event.target.value)}
              >
                {(versions ?? []).map((entry) => (
                  <option key={entry.id} value={entry.id}>
                    {entry.id}
                  </option>
                ))}
                {versions && versions.length === 0 ? (
                  <option value={gameVersion}>{gameVersion}</option>
                ) : null}
              </Select>
            </Field>

            <Field label="Mod loader" htmlFor="new-instance-loader">
              <Select
                id="new-instance-loader"
                value={loader}
                onChange={(event) => setLoader(event.target.value as LoaderKind)}
              >
                {LOADERS.map((kind) => (
                  <option key={kind} value={kind}>
                    {kind}
                  </option>
                ))}
              </Select>
            </Field>
          </div>

          <Field label="Maximum memory" hint="Allocated to the JVM at launch (-Xmx).">
            <NumberInput
              value={maxMemory}
              min={1024}
              max={32768}
              step={512}
              suffix="MB"
              onValueChange={setMaxMemory}
            />
          </Field>

          <label className="flex items-center gap-3 text-sm">
            <Switch checked={installNow} onCheckedChange={setInstallNow} />
            Download and verify the game files now
          </label>
        </div>

        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            onClick={submit}
            disabled={name.trim().length < 2}
            loading={create.isPending}
          >
            <Plus /> Create instance
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
