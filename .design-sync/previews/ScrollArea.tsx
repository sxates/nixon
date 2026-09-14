import { ScrollArea, Separator } from "vinyl";

const meetings = [
  "Demonstration of Shader Effects",
  "UX Critique Session",
  "Meeting 2026-06-25_10-06-35",
  "CM UX/PM Sync (Thursdays)",
  "iOS 27 strategy and resourcing",
  "Journey Builder Spec Review",
  "Vinyl App Audio Test",
  "1:1 — Erin",
];

export function MeetingList() {
  return (
    <ScrollArea className="h-56 w-72 rounded-md border border-border">
      <div className="p-3">
        <h4 className="mb-2 text-sm font-semibold text-foreground">Recent meetings</h4>
        {meetings.map((m) => (
          <div key={m}>
            <div className="py-2 text-sm text-muted-foreground">{m}</div>
            <Separator />
          </div>
        ))}
      </div>
    </ScrollArea>
  );
}
