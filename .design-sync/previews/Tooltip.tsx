import {
  Button,
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "vinyl";

export function Default() {
  return (
    <TooltipProvider>
      <div className="flex items-center justify-center p-8">
        <Tooltip open>
          <TooltipTrigger asChild>
            <Button variant="outline">Enhance</Button>
          </TooltipTrigger>
          <TooltipContent>Improve transcript accuracy with AI</TooltipContent>
        </Tooltip>
      </div>
    </TooltipProvider>
  );
}
