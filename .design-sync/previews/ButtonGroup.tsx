import { ButtonGroup, Button } from "vinyl";

export function Horizontal() {
  return (
    <ButtonGroup>
      <Button variant="outline">Transcript</Button>
      <Button variant="outline">Summary</Button>
      <Button variant="outline">Notes</Button>
    </ButtonGroup>
  );
}

export function Actions() {
  return (
    <ButtonGroup>
      <Button variant="outline">Save</Button>
      <Button variant="outline">Copy</Button>
      <Button variant="outline">Export</Button>
    </ButtonGroup>
  );
}
