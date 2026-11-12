/**
 * One content width for the top-level list pages (Meetings, Action items, People, Ask AI),
 * so moving between them never shifts the content's edges. It is the Meetings month grid's
 * width, the widest of them. A literal class string so Tailwind's scan of src/lib keeps it.
 */
export const PAGE_CONTENT_WIDTH = 'max-w-[1080px]';

/**
 * The page column's side gutter (docs/design-system.md §Spacing). `PageHeader` uses it, so
 * a page that pads its content with the same constant keeps title, tabs and first card on
 * one left edge.
 */
export const PAGE_GUTTER = 'px-4 min-[900px]:px-7';

/**
 * The reading column for documents (meeting details, person details). Narrower than the
 * page column so summary prose stays at a readable measure.
 */
export const DOC_CONTENT_WIDTH = 'max-w-[840px]';
