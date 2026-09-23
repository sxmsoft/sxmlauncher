import { useEffect, useState } from "react";

import { useNavigate } from "react-router-dom";

import { CirclePlay, Link2, Plus } from "lucide-react";
import { useTranslation } from "react-i18next";

import { PageHeader } from "@/components/common/page-header";
import { CreateInstanceDialog } from "@/components/instance/create-instance-dialog";
import { InstanceSettingsDialog } from "@/components/instance/instance-settings-dialog";
import { LaunchPanel } from "@/components/instance/launch-panel";
import { JoinCodeDialog } from "@/components/servers/join-code-dialog";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { CardSkeleton, EmptyState, ErrorState } from "@/components/ui/feedback";
import { useActiveAccount, useInstances, useSelectedInstance } from "@/hooks/queries";

/**
 * Home. The hero launches the selected instance. The library lives on its own
 * route so the rail can point at both.
 */
export function DashboardPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const instances = useInstances();
  const selected = useSelectedInstance();
  const account = useActiveAccount();

  const [createOpen, setCreateOpen] = useState(false);
  useEffect(() => {
    const open = () => setCreateOpen(true);
    window.addEventListener("sxml-new-instance", open);
    return () => window.removeEventListener("sxml-new-instance", open);
  }, []);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [joinOpen, setJoinOpen] = useState(false);

  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        title={t("home.title")}
        description={
          account.data
            ? t("home.signedIn", { user: account.data.username })
            : t("home.signedOut")
        }
        actions={
          <>
            <Button variant="outline" size="sm" className="rounded-full" onClick={() => setJoinOpen(true)}>
              <Link2 className="size-4" /> {t("home.join")}
            </Button>
            <Button size="sm" className="rounded-full" onClick={() => setCreateOpen(true)}>
              <Plus className="size-4" /> {t("home.newInstance")}
            </Button>
          </>
        }
      />

      {instances.isError ? (
        <ErrorState
          title={t("home.loadError")}
          message={instances.error instanceof Error ? instances.error.message : String(instances.error)}
          onRetry={() => void instances.refetch()}
        />
      ) : null}

      {instances.isLoading ? (
        <CardSkeleton className="h-80" />
      ) : selected ? (
        <LaunchPanel
          instance={selected}
          onOpenSettings={() => setSettingsOpen(true)}
          onOpenDetail={() => navigate(`/instances/${selected.id}`)}
        />
      ) : (
        <Card className="overflow-hidden">
          <div className="relative min-h-[320px]">
            <EmptyState
              icon={<CirclePlay />}
              title={t("home.emptyTitle")}
              description={t("home.emptyBody")}
              action={
                <Button onClick={() => setCreateOpen(true)}>
                  <Plus /> {t("home.newInstance")}
                </Button>
              }
            />
          </div>
        </Card>
      )}

      <CreateInstanceDialog open={createOpen} onOpenChange={setCreateOpen} />
      {selected ? (
        <InstanceSettingsDialog instance={selected} open={settingsOpen} onOpenChange={setSettingsOpen} />
      ) : null}
      <JoinCodeDialog open={joinOpen} onOpenChange={setJoinOpen} />
    </div>
  );
}
