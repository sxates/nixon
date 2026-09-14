import {
  Accordion,
  AccordionItem,
  AccordionTrigger,
  AccordionContent,
} from "vinyl";

export function Default() {
  return (
    <Accordion type="single" collapsible defaultValue="transcription" className="w-96">
      <AccordionItem value="transcription">
        <AccordionTrigger>How does transcription work?</AccordionTrigger>
        <AccordionContent>
          Audio is transcribed on-device in real time using Whisper or Parakeet —
          nothing leaves your machine.
        </AccordionContent>
      </AccordionItem>
      <AccordionItem value="speakers">
        <AccordionTrigger>Can it tell speakers apart?</AccordionTrigger>
        <AccordionContent>
          Yes — speaker diarization labels each segment and you can rename speakers.
        </AccordionContent>
      </AccordionItem>
      <AccordionItem value="privacy">
        <AccordionTrigger>Where is my data stored?</AccordionTrigger>
        <AccordionContent>
          Locally, in an on-device database. The only outbound traffic is to your
          chosen summarization model.
        </AccordionContent>
      </AccordionItem>
    </Accordion>
  );
}
