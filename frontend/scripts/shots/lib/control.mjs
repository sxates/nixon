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
      if (q) {
        try {
          q.res(parseReply(line));
        } catch (e) {
          q.rej(e);
        }
      }
    }
  });
  sock.on('error', (e) => {
    while (queue.length) queue.shift().rej(e);
  });
  sock.on('close', () => {
    closed = true;
    while (queue.length) queue.shift().rej(new Error('control socket closed'));
  });
  const ready = new Promise((res, rej) => {
    sock.once('connect', res);
    sock.once('error', rej);
  });
  return {
    ready,
    send: (obj, timeoutMs = 30000) =>
      new Promise((res, rej) => {
        if (closed) {
          rej(new Error('control socket closed'));
          return;
        }
        const t = setTimeout(() => rej(new Error('timeout ' + obj.cmd)), timeoutMs);
        queue.push({
          res: (v) => {
            clearTimeout(t);
            res(v);
          },
          rej: (e) => {
            clearTimeout(t);
            rej(e);
          },
        });
        sock.write(frame(obj));
      }),
    close: () => sock.end(),
  };
}
