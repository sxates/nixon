import { Alert, AlertTitle, AlertDescription } from "vinyl";
import { Info, AlertTriangle } from "lucide-react";

export function Default() {
  return (
    <Alert className="w-96">
      <Info className="h-4 w-4" />
      <AlertTitle>Recording locally</AlertTitle>
      <AlertDescription>
        Audio and transcripts stay on this device. Nothing is uploaded.
      </AlertDescription>
    </Alert>
  );
}

export function Destructive() {
  return (
    <Alert variant="destructive" className="w-96">
      <AlertTriangle className="h-4 w-4" />
      <AlertTitle>Screen-recording permission needed</AlertTitle>
      <AlertDescription>
        System audio can&apos;t be captured until you grant Screen Recording access.
      </AlertDescription>
    </Alert>
  );
}
