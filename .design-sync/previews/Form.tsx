import { useForm } from "react-hook-form";
import {
  Form,
  FormField,
  FormItem,
  FormLabel,
  FormControl,
  FormDescription,
  FormMessage,
  Input,
  Button,
} from "vinyl";

export function Default() {
  const form = useForm({
    defaultValues: { title: "Demonstration of Shader Effects" },
  });
  return (
    <Form {...form}>
      <form className="flex w-80 flex-col gap-4">
        <FormField
          control={form.control}
          name="title"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Meeting title</FormLabel>
              <FormControl>
                <Input placeholder="Untitled meeting" {...field} />
              </FormControl>
              <FormDescription>Shown in your meeting list.</FormDescription>
              <FormMessage />
            </FormItem>
          )}
        />
        <Button type="submit" variant="blue" className="self-start">
          Save
        </Button>
      </form>
    </Form>
  );
}
