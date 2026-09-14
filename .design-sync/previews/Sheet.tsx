import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  SheetDescription,
  SheetFooter,
  Button,
  Label,
  Switch,
} from "vinyl";

export function Default() {
  return (
    <Sheet open>
      <SheetContent side="right" className="w-80">
        <SheetHeader>
          <SheetTitle>Recording settings</SheetTitle>
          <SheetDescription>Tune capture for this meeting.</SheetDescription>
        </SheetHeader>
        <div className="flex flex-col gap-4 py-4">
          <div className="flex items-center justify-between">
            <Label htmlFor="sys">Capture system audio</Label>
            <Switch id="sys" defaultChecked />
          </div>
          <div className="flex items-center justify-between">
            <Label htmlFor="spk">Identify speakers</Label>
            <Switch id="spk" defaultChecked />
          </div>
        </div>
        <SheetFooter>
          <Button variant="blue">Done</Button>
        </SheetFooter>
      </SheetContent>
    </Sheet>
  );
}
