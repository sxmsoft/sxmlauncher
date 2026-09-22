import { Component, type ErrorInfo, type ReactNode } from "react";

import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";

/**
 * Last line of defence.
 *
 * A render-time crash in one page should not take the whole launcher down with
 * it (a game or a hosted world may still be running behind this window), so the
 * boundary catches, reports the component stack, and offers a reload.
 */
export class ErrorBoundary extends Component<
  { children: ReactNode },
  { error: Error | null; info: string | null }
> {
  state: { error: Error | null; info: string | null } = { error: null, info: null };

  static getDerivedStateFromError(error: Error) {
    return { error, info: null };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    this.setState({ error, info: info.componentStack ?? null });
    console.error("[sxmlauncher] render error", error, info.componentStack);
  }

  render() {
    const { error, info } = this.state;
    if (!error) return this.props.children;

    return (
      <div className="app-aurora flex h-full items-center justify-center p-8">
        <Card className="flex max-w-lg flex-col gap-4 p-6">
          <div className="flex flex-col gap-1">
            <h1 className="text-lg font-semibold">The launcher hit a problem</h1>
            <p className="text-muted-foreground text-sm">
              Background work (downloads, a hosted world, a running game) is unaffected.
              Reloading the interface is usually enough.
            </p>
          </div>
          <pre className="max-h-40 overflow-auto rounded-lg border border-white/8 bg-black/40 p-3 text-[11px]">
            {error.message}
            {info ? `\n${info}` : ""}
          </pre>
          <div className="flex gap-2">
            <Button onClick={() => window.location.reload()}>Reload interface</Button>
            <Button variant="ghost" onClick={() => this.setState({ error: null, info: null })}>
              Try again
            </Button>
          </div>
        </Card>
      </div>
    );
  }
}
