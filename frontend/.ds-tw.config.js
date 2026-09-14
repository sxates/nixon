// Temp Tailwind config for design-sync CSS compile.
// Extends the app's real config but scopes content to the synced primitives +
// authored previews, and safelists the semantic token + layout utilities the
// design agent needs when composing its own markup (purge would drop them).
const base = require("./tailwind.config.js");
module.exports = {
  ...base,
  content: [
    "./src/components/ui/*.tsx",
    "./src/lib/utils.ts",
    "../.design-sync/previews/**/*.tsx",
  ],
  safelist: [
    { pattern: /^(bg|text|border|ring|fill|stroke)-(background|foreground|primary|secondary|muted|accent|destructive|card|popover|border|input|ring)$/ },
    { pattern: /^(bg|text|border|ring)-(brand|record)(-foreground)?$/ },
    { pattern: /^(bg|text|border)-(brand|record)\/(10|20|90)$/ },
    { pattern: /^font-(sans|serif)$/ },
    { pattern: /^(bg|text)-(primary|secondary|muted|accent|destructive|card|popover)-foreground$/ },
    { pattern: /^(flex|inline-flex|grid|hidden|block|inline-block)$/ },
    { pattern: /^(flex-row|flex-col|flex-wrap|items-(start|center|end|stretch|baseline)|justify-(start|center|end|between|around|evenly))$/ },
    { pattern: /^grid-cols-[1-6]$/ },
    { pattern: /^gap-(0|1|2|3|4|5|6|8|10|12)$/ },
    { pattern: /^(p|px|py|pt|pb|pl|pr|m|mx|my|mt|mb|ml|mr)-(0|1|2|3|4|5|6|8|10|12|16)$/ },
    { pattern: /^w-(full|auto|fit|screen|8|10|12|16|24|32|40|48|56|64|72|80|96)$/ },
    { pattern: /^h-(full|auto|fit|screen|8|9|10|12|16|24|32)$/ },
    { pattern: /^max-w-(xs|sm|md|lg|xl|2xl)$/ },
    { pattern: /^rounded(-(sm|md|lg|xl|full))?$/ },
    { pattern: /^text-(xs|sm|base|lg|xl|2xl|3xl)$/ },
    { pattern: /^font-(normal|medium|semibold|bold)$/ },
    { pattern: /^(border|border-0|border-2|shadow|shadow-sm|shadow-md|shadow-lg)$/ },
  ],
};
