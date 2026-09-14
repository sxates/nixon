import {
  Popover,
  PopoverTrigger,
  PopoverContent,
  Button,
  Label,
  Input,
} from "vinyl";

export function Default() {
  return (
    <Popover open>
      <PopoverTrigger asChild>
        <Button variant="outline">Rename speaker</Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-72">
        <div className="flex flex-col gap-3">
          <div className="space-y-1">
            <h4 className="text-sm font-semibold text-foreground">Speaker name</h4>
            <p className="text-sm text-muted-foreground">
              Pick an attendee or type a name.
            </p>
          </div>
          <div className="flex flex-col gap-2">
            <Label htmlFor="spk">Name</Label>
            <Input id="spk" defaultValue="Speaker 1" />
          </div>
        </div>
      </PopoverContent>
    </Popover>
  );
}
