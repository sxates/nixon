import { Tabs, TabsContent, TabsList, TabsTrigger } from "vinyl";

export function Default() {
  return (
    <Tabs defaultValue="transcript" className="w-80">
      <TabsList className="grid w-full grid-cols-2">
        <TabsTrigger value="transcript">Transcript</TabsTrigger>
        <TabsTrigger value="notes">My Notes</TabsTrigger>
      </TabsList>
      <TabsContent value="transcript">
        <p className="px-1 py-3 text-sm text-muted-foreground">
          [00:13] This is me talking while this thing is recording.
        </p>
      </TabsContent>
      <TabsContent value="notes">
        <p className="px-1 py-3 text-sm text-muted-foreground">
          Enter text or type &apos;/&apos; for commands
        </p>
      </TabsContent>
    </Tabs>
  );
}
