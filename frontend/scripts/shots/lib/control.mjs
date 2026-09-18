// control.mjs — client for the debug-only dev-control listener (specs/0060).
import net from 'node:net';
import { readFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';

export const frame = (obj) => JSON.stringify(obj) + '\n';
export function parseReply(line) {
  const r = JSON.parse(line);
  if (!r.ok) throw new Error(r.error || 'control error');
  return r;
}
export function readPort() {
  try {
    return Number(
      readFileSync(
        join(homedir(), 'Library', 'Application Support', 'ai.vinyl.app.debug', 'dev-control.port'),
        'utf8',
      ).trim(),
    );
  } catch {
    throw new Error('dev-control.port not found; launch the dev app with ./dev-nixon.sh --demo (or --control) first');
  }
}

// A stale port file (dev app killed/crashed without cleanup) or no dev app running at all
// both surface as ECONNREFUSED — `port not found` (readPort's error) doesn't cover this
// case, since the file is still there and reads fine.
function mapConnError(e) {
  if (e && e.code === 'ECONNREFUSED') {
    return new Error('dev-control.port is stale or Dev Nixon is not running with --demo/--control');
  }
  return e;
}

export function connect(port) {
  const sock = net.connect({ host: '127.0.0.1', port });
  let buf = '';
  let closed = false;
  const queue = [];
  sock.on('data', (d) => {
    buf += d.toString();
    let i;
    while ((i = buf.indexOf('\n')) >= 0) {
      const line = buf.slice(0, i + 1);
      buf = buf.slice(i + 1);
      const q = queue.shift();
      if (!q) continue;
      // `q` is the entry a client-side timeout already rejected (marked `dead` below):
      // this line is its late reply. Shift it off the queue (so the *next* line goes to
      // the next real request) but don't resolve/reject an already-settled promise.
      if (q.dead) continue;
      try {
        q.res(parseReply(line));
      } catch (e) {
        q.rej(e);
      }
    }
  });
  sock.on('error', (e) => {
    const mapped = mapConnError(e);
    while (queue.length) queue.shift().rej(mapped);
  });
  sock.on('close', () => {
    closed = true;
    while (queue.length) queue.shift().rej(new Error('control socket closed'));
  });
  const ready = new Promise((res, rej) => {
    sock.once('connect', res);
    sock.once('error', (e) => rej(mapConnError(e)));
  });
  return {
    ready,
    send: (obj, timeoutMs = 30000) =>
      new Promise((res, rej) => {
        if (closed) {
          rej(new Error('control socket closed'));
          return;
        }
        const entry = {
          res: (v) => {
            clearTimeout(t);
            res(v);
          },
          rej: (e) => {
            clearTimeout(t);
            rej(e);
          },
        };
        // On timeout the entry stays at the queue head (not spliced out) — marking it
        // `dead` tells the `data` handler above to shift-and-drop its late reply instead
        // of resolving it onto whatever the *next* queued request turns out to be.
        const t = setTimeout(() => {
          entry.dead = true;
          entry.rej(new Error('timeout ' + obj.cmd));
        }, timeoutMs);
        queue.push(entry);
        sock.write(frame(obj));
      }),
    close: () => sock.end(),
  };
}
