import { Button } from "vinyl";

export function Variants() {
  return (
    <div className="flex flex-wrap items-center gap-3">
      <Button>Record</Button>
      <Button variant="secondary">Cancel</Button>
      <Button variant="outline">Open</Button>
      <Button variant="ghost">Dismiss</Button>
      <Button variant="destructive">Delete</Button>
      <Button variant="link">Learn more</Button>
    </div>
  );
}

export function BrandVariants() {
  return (
    <div className="flex flex-wrap items-center gap-3">
      <Button variant="blue">Join &amp; Record</Button>
      <Button variant="green">Summarize</Button>
      <Button variant="red">Stop</Button>
      <Button variant="gray">Settings</Button>
    </div>
  );
}

export function Sizes() {
  return (
    <div className="flex flex-wrap items-center gap-3">
      <Button size="sm">Small</Button>
      <Button size="default">Default</Button>
      <Button size="lg">Large</Button>
    </div>
  );
}

export function States() {
  return (
    <div className="flex flex-wrap items-center gap-3">
      <Button>Enabled</Button>
      <Button disabled>Disabled</Button>
      <Button variant="outline" disabled>
        Disabled
      </Button>
    </div>
  );
}
