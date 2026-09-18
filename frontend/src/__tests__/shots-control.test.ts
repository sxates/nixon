import { describe, it, expect, vi } from 'vitest';
import { EventEmitter } from 'node:events';

// A fake `net.Socket`: a plain EventEmitter with the two methods `connect()` calls
// (`write`, `end`) stubbed, so `connect()`'s queued-request/close-handling logic can be
// exercised without a real TCP listener.
class FakeSocket extends EventEmitter {
  write = vi.fn();
  end = vi.fn();
}

let fakeSocket: FakeSocket;
vi.mock('node:net', () => ({
  default: {
    connect: () => fakeSocket,
  },
}));

const { frame, parseReply, connect } = await import('../../scripts/shots/lib/control.mjs');

describe('control protocol', () => {
  it('frames one JSON object per line', () => {
    expect(frame({ cmd: 'ping' })).toBe('{"cmd":"ping"}\n');
  });
  it('parses ok and error replies', () => {
    expect(parseReply('{"ok":true,"ms":3}\n')).toEqual({ ok: true, ms: 3 });
    expect(() => parseReply('{"ok":false,"error":"boom"}\n')).toThrow(/boom/);
  });

  it('rejects a pending request when the socket closes', async () => {
    fakeSocket = new FakeSocket();
    const c = connect(1234);
    const pending = c.send({ cmd: 'ping' });
    fakeSocket.emit('close');
    await expect(pending).rejects.toThrow(/control socket closed/);
  });

  it('rejects send immediately once the socket is already closed', async () => {
    fakeSocket = new FakeSocket();
    const c = connect(1234);
    fakeSocket.emit('close');
    await expect(c.send({ cmd: 'ping' })).rejects.toThrow(/control socket closed/);
    expect(fakeSocket.write).not.toHaveBeenCalled();
  });
});
