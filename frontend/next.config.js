const path = require('path');
const tiptapPmResolveBase = path.dirname(require.resolve('@tiptap/pm/model'));
const resolveFromTiptapPm = (pkg) =>
  require.resolve(pkg, { paths: [tiptapPmResolveBase] });

/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: false, // Disabled for BlockNote compatibility
  output: 'export',
  images: {
    unoptimized: true,
  },
  // Add basePath configuration
  basePath: '',
  assetPrefix: '/',

  // Add webpack configuration for Tauri
  webpack: (config, { isServer, dev }) => {
    // In dev, Next defaults to `eval-source-map`, which inline-embeds a base64 source map in
    // every module's eval() wrapper. That balloons chunks (layout.js hit ~6.4MB) and the macOS
    // WKWebView (Tauri) truncates the oversized response mid-eval -> "SyntaxError: Unexpected
    // EOF" + a blank screen. Disabling source maps in dev keeps modules un-eval-wrapped and the
    // chunk small enough for the webview to load.
    if (dev && !isServer) {
      // Next forcibly reverts `config.devtool` back to 'eval-source-map' in dev. That inlines a
      // base64 source map into every module's eval() wrapper, ballooning layout.js to ~6.4MB —
      // big enough that the macOS WKWebView (Tauri) truncates the response mid-eval -> blank
      // screen + "SyntaxError: Unexpected EOF". Pin devtool off via a no-op setter so Next's
      // revert can't take, keeping the chunk small enough for the webview to load.
      Object.defineProperty(config, 'devtool', {
        configurable: true,
        enumerable: true,
        get: () => false,
        set: () => {},
      });

      // devtool=false alone only got layout.js from ~6.4MB down to ~3MB, and the macOS
      // WKWebView (Tauri) STILL truncates that 3MB response mid-load -> "Unexpected end of
      // script (layout.js)" + ChunkLoadError + blank page. The server is fine (curl pulls the
      // full body on both IPv4/IPv6 in <10ms; node --check passes) — this is WKWebView's
      // NSURLSession dropping large localhost script bodies above a size threshold. It is a
      // SIZE problem, not a slow-load one, so bumping chunkLoadTimeout would not help. The fix
      // is to make sure no single dev chunk is large: force aggressive splitChunks so every
      // emitted chunk stays well under the threshold the webview chokes on. Dev-only — prod
      // (output: 'export') ships minified assets over the tauri:// protocol from disk and is
      // unaffected.
      config.optimization = config.optimization || {};
      config.optimization.splitChunks = {
        chunks: 'all',
        // ~244KB per chunk: small enough that WKWebView reliably loads each one whole.
        maxSize: 244 * 1024,
        minSize: 20 * 1024,
        cacheGroups: {
          // Split node_modules into many small vendor chunks instead of one giant blob.
          vendor: {
            test: /[\\/]node_modules[\\/]/,
            name(module) {
              const match = module.context
                ? module.context.match(/[\\/]node_modules[\\/](.*?)(?:[\\/]|$)/)
                : null;
              const pkg = match ? match[1].replace('@', '').replace(/[\\/]/g, '-') : 'vendor';
              return `vendor-${pkg}`;
            },
            priority: 10,
            reuseExistingChunk: true,
          },
          default: {
            minChunks: 1,
            priority: -20,
            reuseExistingChunk: true,
          },
        },
      };
    }
    if (!isServer) {
      config.resolve.fallback = {
        ...config.resolve.fallback,
        fs: false,
        path: false,
        os: false,
      };

      // Keep ProseMirror single-instanced for BlockNote/Tiptap.
      config.resolve.alias = {
        ...config.resolve.alias,
        '@blocknote/core$': require.resolve('@blocknote/core'),
        '@blocknote/react$': require.resolve('@blocknote/react'),
        '@blocknote/shadcn$': require.resolve('@blocknote/shadcn'),
        'prosemirror-model': resolveFromTiptapPm('prosemirror-model'),
        'prosemirror-state': resolveFromTiptapPm('prosemirror-state'),
        'prosemirror-view': resolveFromTiptapPm('prosemirror-view'),
        'prosemirror-transform': resolveFromTiptapPm('prosemirror-transform'),
        'prosemirror-tables': resolveFromTiptapPm('prosemirror-tables'),
        'prosemirror-schema-list': resolveFromTiptapPm('prosemirror-schema-list'),
        'prosemirror-keymap': resolveFromTiptapPm('prosemirror-keymap'),
        'prosemirror-commands': resolveFromTiptapPm('prosemirror-commands'),
        'prosemirror-history': resolveFromTiptapPm('prosemirror-history'),
        'prosemirror-inputrules': resolveFromTiptapPm('prosemirror-inputrules'),
        'prosemirror-gapcursor': resolveFromTiptapPm('prosemirror-gapcursor'),
        'prosemirror-dropcursor': resolveFromTiptapPm('prosemirror-dropcursor'),
      };
    }
    return config;
  },
}

module.exports = nextConfig
