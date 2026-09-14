import { Label, Input, Switch } from "vinyl";

export function WithInput() {
  return (
    <div className="flex w-72 flex-col gap-2">
      <Label htmlFor="title">Meeting title</Label>
      <Input id="title" placeholder="Untitled meeting" />
    </div>
  );
}

export function WithControl() {
  return (
    <div className="flex w-72 items-center justify-between">
      <Label htmlFor="diarize">Identify speakers</Label>
      <Switch id="diarize" defaultChecked />
    </div>
  );
}
