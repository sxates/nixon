import { VisuallyHidden, Button } from "vinyl";
import { Mic } from "lucide-react";

export function IconButton() {
  return (
    <div className="flex items-center gap-3">
      <Button size="icon" variant="outline">
        <Mic className="h-4 w-4" />
        <VisuallyHidden>Start recording</VisuallyHidden>
      </Button>
      <span className="text-sm text-muted-foreground">
        Icon-only button with a screen-reader-only label
      </span>
    </div>
  );
}
