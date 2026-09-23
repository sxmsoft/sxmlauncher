import { ActivityFeed } from "@/components/jobs/activity-panel";

/** Full-page activity. The title-bar button still opens the same feed as a drawer. */
export function ActivityPage() {
  return <ActivityFeed embedded />;
}
