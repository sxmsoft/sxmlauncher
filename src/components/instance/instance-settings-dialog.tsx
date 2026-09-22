import { useEffect, useState } from "react";

import { Save, Trash } from "lucide-react";

import { Button } from "@/components/ui/button";
import { ConfirmDialog, Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Separator } from "@/components/ui/feedback";
import { Field, Input, Select } from "@/components/ui/input";
import { NumberInput, Switch } from "@/components/ui/switch";
import { useDeleteInstance, useJavaRuntimes, useUpdateInstance } from "@/hooks/queries";
import { clampMemory } from "@/types/system";
import type { Instance } from "@/types/instance";

/**
 * Instance settings.
 *
 * Everything here is per-instance by design: memory, resolution, JVM flags and
 * the Java runtime. A modpack that needs Java 17 and a vanilla instance that
 * wants Java 21 can coexist on the same machine.
 */
export function InstanceSettingsDialog({
  instance,
  open,
  onOpenChange,
}: {
  instance: Instance;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const update = useUpdateInstance();
  const remove = useDeleteInstance();
  const { data: runtimes } = useJavaRuntimes();

  const [name, setName] = useState(instance.name);
  const [description, setDescription] = useState(instance.description);
  const [minMb, setMinMb] = useState(instance.memory.minMb);
  const [maxMb, setMaxMb] = useState(instance.memory.maxMb);
  const [width, setWidth] = useState(instance.resolution.width);
  const [height, setHeight] = useState(instance.resolution.height);
  const [fullscreen, setFullscreen] = useState(instance.resolution.fullscreen);
  const [javaMajor, setJavaMajor] = useState<number | null>(instance.java.preferredMajor);
  const [overridePath, setOverridePath] = useState(instance.java.overridePath ?? "");
  const [autoJava, setAutoJava] = useState(instance.java.autoDownload);
  const [jvmArgs, setJvmArgs] = useState(instance.java.jvmArgs.join(" "));
  const [gameArgs, setGameArgs] = useState(instance.gameArgs.join(" "));
  const [confirmDelete, setConfirmDelete] = useState(false);

  // Re-seed the form whenever a different instance is opened.
  useEffect(() => {
    setName(instance.name);
    setDescription(instance.description);
    setMinMb(instance.memory.minMb);
    setMaxMb(instance.memory.maxMb);
    setWidth(instance.resolution.width);
    setHeight(instance.resolution.height);
    setFullscreen(instance.resolution.fullscreen);
    setJavaMajor(instance.java.preferredMajor);
    setOverridePath(instance.java.overridePath ?? "");
    setAutoJava(instance.java.autoDownload);
    setJvmArgs(instance.java.jvmArgs.join(" "));
    setGameArgs(instance.gameArgs.join(" "));
  }, [instance]);

  const splitArgs = (value: string) =>
    value
      .split(/\s+/)
      .map((part) => part.trim())
      .filter(Boolean);

  const save = () => {
    const safeMin = clampMemory(minMb);
    const safeMax = Math.max(safeMin, clampMemory(maxMb));
    update.mutate(
      {
        id: instance.id,
        name: name.trim(),
        description: description.trim(),
        memory: { minMb: safeMin, maxMb: safeMax },
        resolution: { width: Math.max(640, width), height: Math.max(480, height), fullscreen },
        java: {
          overridePath: overridePath.trim() || null,
          preferredMajor: javaMajor,
          autoDownload: autoJava,
          jvmArgs: splitArgs(jvmArgs),
        },
        gameArgs: splitArgs(gameArgs),
      },
      { onSuccess: () => onOpenChange(false) },
    );
  };

  return (
    <>
      <Dialog open={open} onOpenChange={onOpenChange}>
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle>{instance.name} · settings</DialogTitle>
          </DialogHeader>

          <div className="flex flex-col gap-5">
            <section className="flex flex-col gap-4">
              <Field label="Name" htmlFor="inst-name">
                <Input id="inst-name" value={name} onChange={(event) => setName(event.target.value)} />
              </Field>
              <Field label="Description" htmlFor="inst-desc">
                <Input
                  id="inst-desc"
                  value={description}
                  onChange={(event) => setDescription(event.target.value)}
                />
              </Field>
            </section>

            <Separator />

            <section className="grid gap-4 sm:grid-cols-2">
              <Field label="Minimum memory" hint="Initial heap (-Xms).">
                <NumberInput value={minMb} min={512} max={32768} step={512} suffix="MB" onValueChange={setMinMb} />
              </Field>
              <Field label="Maximum memory" hint="Heap ceiling (-Xmx).">
                <NumberInput value={maxMb} min={512} max={32768} step={512} suffix="MB" onValueChange={setMaxMb} />
              </Field>
            </section>

            <Separator />

            <section className="grid gap-4 sm:grid-cols-3">
              <Field label="Window width">
                <NumberInput value={width} min={640} max={7680} step={10} onValueChange={setWidth} />
              </Field>
              <Field label="Window height">
                <NumberInput value={height} min={480} max={4320} step={10} onValueChange={setHeight} />
              </Field>
              <label className="flex items-end gap-3 pb-2 text-sm">
                <Switch checked={fullscreen} onCheckedChange={setFullscreen} />
                Fullscreen
              </label>
            </section>

            <Separator />

            <section className="grid gap-4 sm:grid-cols-2">
              <Field
                label="Java runtime"
                hint="Leave on auto to use the version this Minecraft release needs."
              >
                <Select
                  value={javaMajor ?? "auto"}
                  onChange={(event) =>
                    setJavaMajor(event.target.value === "auto" ? null : Number(event.target.value))
                  }
                >
                  <option value="auto">Automatic (Java {instance.requiredJavaMajor})</option>
                  {(runtimes ?? []).map((runtime) => (
                    <option key={runtime.path} value={runtime.major}>
                      Java {runtime.major} · {runtime.vendor} {runtime.isManaged ? "(managed)" : ""}
                    </option>
                  ))}
                </Select>
              </Field>
              <Field label="Java executable" hint="Overrides the detected runtime (path to `java`).">
                <Input
                  value={overridePath}
                  onChange={(event) => setOverridePath(event.target.value)}
                  placeholder="auto-detected"
                />
              </Field>
              <label className="flex items-center gap-3 text-sm sm:col-span-2">
                <Switch checked={autoJava} onCheckedChange={setAutoJava} />
                Download the required Java runtime automatically
              </label>
            </section>

            <Separator />

            <section className="flex flex-col gap-4">
              <Field label="JVM arguments" hint="Space separated. Use -Xmx/-Xms via the memory fields above.">
                <Input
                  value={jvmArgs}
                  onChange={(event) => setJvmArgs(event.target.value)}
                  placeholder="-XX:+UseG1GC -Dfile.encoding=UTF-8"
                />
              </Field>
              <Field label="Game arguments" hint="Passed to Minecraft itself.">
                <Input
                  value={gameArgs}
                  onChange={(event) => setGameArgs(event.target.value)}
                  placeholder="--fullscreen"
                />
              </Field>
            </section>
          </div>

          <DialogFooter className="justify-between">
            <Button
              variant="ghost"
              className="text-[var(--destructive)]"
              onClick={() => setConfirmDelete(true)}
            >
              <Trash /> Remove instance
            </Button>
            <div className="flex gap-2">
              <Button variant="ghost" onClick={() => onOpenChange(false)}>
                Cancel
              </Button>
              <Button onClick={save} loading={update.isPending}>
                <Save /> Save
              </Button>
            </div>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <ConfirmDialog
        open={confirmDelete}
        onOpenChange={setConfirmDelete}
        title={`Remove “${instance.name}”?`}
        description="The instance folder, its mods and its worlds are deleted from disk."
        confirmLabel="Remove instance"
        destructive
        busy={remove.isPending}
        onConfirm={() =>
          remove.mutate(
            { id: instance.id, deleteFiles: true },
            {
              onSuccess: () => {
                setConfirmDelete(false);
                onOpenChange(false);
              },
            },
          )
        }
      />
    </>
  );
}
