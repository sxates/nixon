import { Separator } from "vinyl";

export function Horizontal() {
  return (
    <div className="w-72">
      <div className="space-y-1">
        <h4 className="text-sm font-semibold text-foreground">Transcript</h4>
        <p className="text-sm text-muted-foreground">On-device, real-time.</p>
      </div>
      <Separator className="my-4" />
      <div className="space-y-1">
        <h4 className="text-sm font-semibold text-foreground">Summary</h4>
        <p className="text-sm text-muted-foreground">AI-enhanced notes.</p>
      </div>
    </div>
  );
}

export function Vertical() {
  return (
    <div className="flex h-6 items-center gap-3 text-sm text-foreground">
      <span>Transcript</span>
      <Separator orientation="vertical" />
      <span>Summary</span>
      <Separator orientation="vertical" />
      <span>Notes</span>
    </div>
  );
}
