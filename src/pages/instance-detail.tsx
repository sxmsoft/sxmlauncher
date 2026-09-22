import { useState } from "react";

import { useNavigate, useParams } from "react-router-dom";

import { ArrowLeft, Settings } from "lucide-react";

import { PageHeader } from "@/components/common/page-header";
import { InstanceSettingsDialog } from "@/components/instance/instance-settings-dialog";
import { LaunchPanel } from "@/components/instance/launch-panel";
import { ModList } from "@/components/instance/mod-list";
import { ModBrowser } from "@/components/mods/mod-browser";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { ErrorState, Skeleton, Stat } from "@/components/ui/feedback";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useInstance, useJavaRuntimes } from "@/hooks/queries";
import { formatBytes, formatDuration, formatRelative } from "@/lib/utils";

/**
 * Single instance view.
 *
 * The page a player lives on while tuning a setup: launch, host, browse mods
 * *for this version and loader*, and toggle what is installed.
 */
export function InstanceDetailPage() {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const { data: instance, isLoading, isError, error, refetch } = useInstance(id ?? null);
  const { data: runtimes } = useJavaRuntimes();
  const [settingsOpen, setSettingsOpen] = useState(false);

  if (isLoading) {
    return (
      <div className="flex flex-col gap-4">
        <Skeleton className="h-8 w-64" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  if (isError || !instance) {
    return (
      <ErrorState
        title="Instance not found"
        message={error instanceof Error ? error.message : "That instance no longer exists."}
        onRetry={() => void refetch()}
      />
    );
  }

  const java = (runtimes ?? []).find(
    (runtime) => runtime.major === (instance.java.preferredMajor ?? instance.requiredJavaMajor),
  );

  return (
    <div className="flex flex-col gap-5">
      <PageHeader
        title={instance.name}
        description={instance.description || "No description"}
        actions={
          <>
            <Button variant="ghost" size="sm" onClick={() => navigate("/")}>
              <ArrowLeft className="size-4" /> Play
            </Button>
            <Button variant="outline" size="sm" onClick={() => setSettingsOpen(true)}>
              <Settings className="size-4" /> Settings
            </Button>
          </>
        }
      />

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        <Card className="p-4">
          <Stat label="Version" value={`${instance.gameVersion} · ${instance.loader.kind}`} />
        </Card>
        <Card className="p-4">
          <Stat
            label="Java"
            value={java ? `Java ${java.major} · ${java.vendor}` : `Java ${instance.requiredJavaMajor} required`}
          />
        </Card>
        <Card className="p-4">
          <Stat label="Disk" value={formatBytes(instance.sizeBytes)} />
        </Card>
        <Card className="p-4">
          <Stat
            label="Playtime"
            value={`${formatDuration(instance.totalPlaytimeSecs)} · ${instance.launchCount} launches`}
          />
        </Card>
      </div>

      <LaunchPanel
        instance={instance}
        onOpenSettings={() => setSettingsOpen(true)}
        onOpenDetail={() => undefined}
      />

      <Tabs defaultValue="mods">
        <TabsList>
          <TabsTrigger value="mods">
            Installed mods <Badge variant="outline">{instance.modCount}</Badge>
          </TabsTrigger>
          <TabsTrigger value="browse">Browse & install</TabsTrigger>
          <TabsTrigger value="pack">Pack source</TabsTrigger>
        </TabsList>

        <TabsContent value="mods">
          <ModList instanceId={instance.id} onBrowseMods={() => navigate("/modpacks")} />
        </TabsContent>

        <TabsContent value="browse">
          <ModBrowser instance={instance} />
        </TabsContent>

        <TabsContent value="pack">
          <Card>
            <CardHeader>
              <CardTitle>Where this instance came from</CardTitle>
            </CardHeader>
            <CardContent className="flex flex-col gap-3">
              {instance.sourcePack ? (
                <>
                  <div className="flex flex-wrap items-center gap-2">
                    <Badge variant="primary">{instance.sourcePack.source}</Badge>
                    <span className="text-sm font-medium">{instance.sourcePack.name}</span>
                    <Badge variant="outline">{instance.sourcePack.versionNumber}</Badge>
                  </div>
                  <div className="text-muted-foreground flex flex-wrap gap-4 text-xs">
                    <span>project {instance.sourcePack.projectId}</span>
                    <span>version {instance.sourcePack.versionId}</span>
                    <span>updated {formatRelative(instance.updatedAt)}</span>
                  </div>
                  <p className="text-muted-foreground text-xs leading-relaxed">
                    Updating the pack re-resolves every file against the current manifest and
                    verifies hashes; mods you added yourself are left untouched.
                  </p>
                </>
              ) : (
                <p className="text-muted-foreground text-sm">
                  This instance was created by hand, so there is no pack to track. Mods
                  installed through the browser are still recorded per project.
                </p>
              )}
            </CardContent>
          </Card>
        </TabsContent>
      </Tabs>

      <InstanceSettingsDialog
        instance={instance}
        open={settingsOpen}
        onOpenChange={setSettingsOpen}
      />
    </div>
  );
}
