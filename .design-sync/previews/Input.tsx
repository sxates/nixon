import { Input } from "vinyl";

export function Default() {
  return (
    <div className="flex w-72 flex-col gap-2">
      <label className="text-sm font-medium text-foreground">Meeting title</label>
      <Input placeholder="Untitled meeting" />
    </div>
  );
}

export function States() {
  return (
    <div className="flex w-72 flex-col gap-3">
      <Input placeholder="Search meeting content…" />
      <Input defaultValue="Q3 Planning Sync" />
      <Input type="email" placeholder="you@company.com" />
      <Input placeholder="Disabled" disabled />
    </div>
  );
}
