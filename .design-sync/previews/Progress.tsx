import { Progress } from "vinyl";

export function Values() {
  return (
    <div className="flex w-80 flex-col gap-5">
      <div className="flex flex-col gap-2">
        <span className="text-sm text-muted-foreground">Downloading model — 25%</span>
        <Progress value={25} />
      </div>
      <div className="flex flex-col gap-2">
        <span className="text-sm text-muted-foreground">Transcribing — 60%</span>
        <Progress value={60} />
      </div>
      <div className="flex flex-col gap-2">
        <span className="text-sm text-muted-foreground">Complete — 100%</span>
        <Progress value={100} />
      </div>
    </div>
  );
}
