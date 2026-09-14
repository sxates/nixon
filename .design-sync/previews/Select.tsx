import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "vinyl";

export function Language() {
  return (
    <div className="flex w-64 flex-col gap-2">
      <label className="text-sm font-medium text-foreground">Language</label>
      <Select defaultValue="auto">
        <SelectTrigger>
          <SelectValue placeholder="Select language" />
        </SelectTrigger>
        <SelectContent>
          <SelectGroup>
            <SelectLabel>Detection</SelectLabel>
            <SelectItem value="auto">Auto-detect</SelectItem>
          </SelectGroup>
          <SelectGroup>
            <SelectLabel>Languages</SelectLabel>
            <SelectItem value="en">English</SelectItem>
            <SelectItem value="es">Spanish</SelectItem>
            <SelectItem value="fr">French</SelectItem>
            <SelectItem value="de">German</SelectItem>
          </SelectGroup>
        </SelectContent>
      </Select>
    </div>
  );
}

export function Model() {
  return (
    <div className="flex w-64 flex-col gap-2">
      <label className="text-sm font-medium text-foreground">Transcription model</label>
      <Select defaultValue="whisper-large">
        <SelectTrigger>
          <SelectValue placeholder="Select model" />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="whisper-large">Whisper Large v3</SelectItem>
          <SelectItem value="whisper-base">Whisper Base</SelectItem>
          <SelectItem value="parakeet">Parakeet</SelectItem>
        </SelectContent>
      </Select>
    </div>
  );
}
