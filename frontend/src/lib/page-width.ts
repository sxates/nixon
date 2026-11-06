/**
 * One content width for the top-level list pages (Meetings, Action items, People, Ask AI),
 * so moving between them never shifts the content's edges. It is the Meetings month grid's
 * width, the widest of them. A literal class string so Tailwind's scan of src/lib keeps it.
 */
export const PAGE_CONTENT_WIDTH = 'max-w-[1080px]';
