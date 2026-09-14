import {
  Command,
  CommandInput,
  CommandList,
  CommandEmpty,
  CommandGroup,
  CommandItem,
  CommandSeparator,
  CommandShortcut,
} from "vinyl";
import { Mic, Upload, Search, Settings } from "lucide-react";

export function Palette() {
  return (
    <Command className="w-80 rounded-lg border border-border shadow-md">
      <CommandInput placeholder="Type a command or search…" />
      <CommandList>
        <CommandEmpty>No results found.</CommandEmpty>
        <CommandGroup heading="Actions">
          <CommandItem>
            <Mic className="mr-2 h-4 w-4" />
            Start recording
            <CommandShortcut>⌘⇧R</CommandShortcut>
          </CommandItem>
          <CommandItem>
            <Upload className="mr-2 h-4 w-4" />
            Import audio
          </CommandItem>
        </CommandGroup>
        <CommandSeparator />
        <CommandGroup heading="Navigation">
          <CommandItem>
            <Search className="mr-2 h-4 w-4" />
            Search meetings
          </CommandItem>
          <CommandItem>
            <Settings className="mr-2 h-4 w-4" />
            Settings
          </CommandItem>
        </CommandGroup>
      </CommandList>
    </Command>
  );
}
