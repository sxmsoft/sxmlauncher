import { useState } from "react";

import { useNavigate } from "react-router-dom";

import { CirclePlay, FolderSearch, Link2, Plus } from "lucide-react";

import { AccountMenu } from "@/components/account/account-menu";
import { PageHeader } from "@/components/common/page-header";
import { CreateInstanceDialog } from "@/components/instance/create-instance-dialog";
import { InstanceCard } from "@/components/instance/instance-card";
import { InstanceSettingsDialog } from "@/components/instance/instance-settings-dialog";
import { LaunchPanel } from "@/components/instance/launch-panel";
import { JoinCodeDialog } from "@/components/servers/join-code-dialog";
import { MySessions } from "@/components/servers/my-sessions";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { CardSkeleton, EmptyState, ErrorState } from "@/components/ui/feedback";
import { useActiveAccount, useImportInstance, useInstances } from "@/hooks/queries";
import { useSelectedInstance } from "@/hooks/queries";
import { useUiStore } from "@/stores/ui";

/**
 * Play page.
 *
 * Two jobs: get the player into a world (launch or host the selected instance)
 * and answer "what can I play?" with the instance grid. Everything else — mods,
 * servers, settings — is one click away in the sidebar.
 */
export function DashboardPage() {
  const navigate = useNavigate();
  const instances = useInstances();
  const selected = useSelectedInstance();
  const account = useActiveAccount();
  const select = useUiStore((state) => state.selectInstance);
  const importFolder = useImportInstance();

  const [createOpen, setCreateOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [joinOpen, setJoinOpen] = useState(false);

  return (
    <div className="flex flex-col gap-5">
      <PageHeader
        title="Play"
        description={
          account.data
            ? `Signed in as ${account.data.username}. Pick an instance, or host a world for friends.`
            : "Add an account to play online, or create an offline profile for singleplayer."
        }
        actions={
          <>
            {!account.data ? <AccountMenu /> : null}
            <Button variant="outline" size="sm" onClick={() => setJoinOpen(true)}>
              <Link2 className="size-4" /> Join with code
            </Button>
            <Button variant="outline" size="sm" onClick={() => importFolder.mutate()} loading={importFolder.isPending}>
              <FolderSearch className="size-4" /> Import folder
            </Button>
            <Button size="sm" onClick={() => setCreateOpen(true)}>
              <Plus className="size-4" /> New instance
            </Button>
          </>
        }
      />

      {instances.isError ? (
        <ErrorState
          title="Could not load instances"
          message={instances.error instanceof Error ? instances.error.message : String(instances.error)}
          onRetry={() => void instances.refetch()}
        />
      ) : null}

      <div className="grid gap-5 xl:grid-cols-[minmax(0,1.6fr)_minmax(320px,1fr)]">
        <div className="flex flex-col gap-5">
          {instances.isLoading ? (
            <CardSkeleton className="h-64" />
          ) : selected ? (
            <LaunchPanel
              instance={selected}
              onOpenSettings={() => setSettingsOpen(true)}
              onOpenDetail={() => navigate(`/instances/${selected.id}`)}
            />
          ) : (
            <Card>
              <CardHeader>
                <CardTitle>No instances yet</CardTitle>
              </CardHeader>
              <CardContent>
                <EmptyState
                  icon={<CirclePlay />}
                  title="Create your first instance"
                  description="An instance is an isolated game environment: its own mods, configs, resource packs and worlds. Vanilla, Fabric, Quilt, Forge and NeoForge are supported."
                  action={
                    <Button onClick={() => setCreateOpen(true)}>
                      <Plus /> New instance
                    </Button>
                  }
                />
              </CardContent>
            </Card>
          )}

          <div>
            <div className="mb-3 flex items-center justify-between">
              <h2 className="text-sm font-semibold tracking-tight">Your instances</h2>
              {instances.data ? (
                <Badge variant="outline">{instances.data.length}</Badge>
              ) : null}
            </div>
            <div className="grid gap-3 sm:grid-cols-2 2xl:grid-cols-3">
              {(instances.data ?? []).map((instance) => (
                <InstanceCard
                  key={instance.id}
                  instance={instance}
                  selected={instance.id === selected?.id}
                  onSelect={() => select(instance.id)}
                  onOpen={() => navigate(`/instances/${instance.id}`)}
                />
              ))}
            </div>
          </div>
        </div>

        <div className="flex flex-col gap-5">
          <MySessions />
        </div>
      </div>

      <CreateInstanceDialog open={createOpen} onOpenChange={setCreateOpen} />
      {selected ? (
        <InstanceSettingsDialog
          instance={selected}
          open={settingsOpen}
          onOpenChange={setSettingsOpen}
        />
      ) : null}
      <JoinCodeDialog open={joinOpen} onOpenChange={setJoinOpen} />
    </div>
  );
}
