import {
  InputGroup,
  InputGroupInput,
  InputGroupAddon,
  InputGroupText,
} from "vinyl";
import { Search } from "lucide-react";

export function WithIcon() {
  return (
    <InputGroup className="w-72">
      <InputGroupAddon align="inline-start">
        <Search className="h-4 w-4 text-muted-foreground" />
      </InputGroupAddon>
      <InputGroupInput placeholder="Search meeting content…" />
    </InputGroup>
  );
}

export function WithText() {
  return (
    <InputGroup className="w-72">
      <InputGroupAddon align="inline-start">
        <InputGroupText>https://</InputGroupText>
      </InputGroupAddon>
      <InputGroupInput placeholder="meeting.zoom.us/j/…" />
    </InputGroup>
  );
}
