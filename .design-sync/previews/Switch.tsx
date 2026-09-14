import { Switch, Label } from "vinyl";

export function States() {
  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center gap-3">
        <Switch defaultChecked />
        <span className="text-sm text-foreground">On</span>
      </div>
      <div className="flex items-center gap-3">
        <Switch />
        <span className="text-sm text-foreground">Off</span>
      </div>
      <div className="flex items-center gap-3">
        <Switch defaultChecked disabled />
        <span className="text-sm text-muted-foreground">Disabled</span>
      </div>
    </div>
  );
}

export function WithLabels() {
  return (
    <div className="flex w-72 flex-col gap-4">
      <div className="flex items-center justify-between">
        <Label htmlFor="auto-record">Auto-record meetings</Label>
        <Switch id="auto-record" defaultChecked />
      </div>
      <div className="flex items-center justify-between">
        <Label htmlFor="diarize">Identify speakers</Label>
        <Switch id="diarize" />
      </div>
    </div>
  );
}
