/**
 * Dev-only logging (spec 0028).
 *
 * Verbose logs that include meeting *content* — transcript snippets, full summary objects —
 * are handy during development but leak that content into the production console (a privacy
 * regression against the product's "content never leaves the machine" promise) and add noise.
 * `devLog` forwards to `console.log` only in development builds and no-ops in production.
 */
export const isDev = process.env.NODE_ENV !== 'production';

export function devLog(...args: unknown[]): void {
  if (isDev) {
    // eslint-disable-next-line no-console
    console.log(...args);
  }
}
