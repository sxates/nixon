import { Textarea, Label } from "vinyl";

export function Default() {
  return (
    <div className="flex w-80 flex-col gap-2">
      <Label htmlFor="ctx">Context for AI summary</Label>
      <Textarea
        id="ctx"
        placeholder="Add context for AI summary. For example people involved, meeting overview, objective etc…"
        rows={4}
      />
    </div>
  );
}

export function States() {
  return (
    <div className="flex w-80 flex-col gap-3">
      <Textarea defaultValue={"Kicked off the shader demo.\nAgent edit-loop discussion."} rows={3} />
      <Textarea placeholder="Disabled" disabled rows={2} />
    </div>
  );
}
