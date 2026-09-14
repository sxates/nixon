// specs/0020 task 9 — shared types for the Templates settings surface.
//
// IPC contract (summary/template_commands.rs). Tauri matches command args by
// camelCase top-level keys ({ templateJson }, { excludeMeetingId }); the template
// JSON *payload* itself uses snake_case serde fields (item_format).

/** Where a listed template comes from (api_list_templates `source`). */
export type TemplateSource = 'builtin' | 'custom' | 'override';

export interface TemplateListItem {
  id: string;
  name: string;
  description: string;
  source: TemplateSource;
  /** User removed it from their list (shipped templates hide, never delete). */
  hidden: boolean;
}

export type SectionFormat = 'paragraph' | 'list' | 'string';

/** One template section — snake_case fields, mirroring the Rust serde schema. */
export interface TemplateSection {
  title: string;
  instruction: string;
  format: SectionFormat;
  item_format?: string | null;
  example_item_format?: string | null;
}

/**
 * Full template details (api_get_template_details). `sections` is the legacy
 * titles-only list; the editor works from `section_details`, the full per-section
 * objects added for specs/0020.
 */
export interface TemplateDetails {
  id?: string;
  name: string;
  description: string;
  sections: string[];
  section_details: TemplateSection[];
}

export const SECTION_FORMATS: Array<{ value: SectionFormat; label: string }> = [
  { value: 'paragraph', label: 'Paragraph' },
  { value: 'list', label: 'List' },
  { value: 'string', label: 'Single line' },
];

export const SOURCE_LABEL: Record<TemplateSource, string> = {
  builtin: 'Built-in',
  custom: 'Custom',
  override: 'Edited',
};
