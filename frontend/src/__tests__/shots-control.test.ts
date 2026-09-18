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

  it('drops a late reply to a timed-out request instead of resolving the next one onto it', async () => {
    vi.useFakeTimers();
    try {
      fakeSocket = new FakeSocket();
      const c = connect(1234);

      // Request A times out client-side after 10ms — nobody ever writes a reply line
      // for it, simulating a command the server never answered.
      const a = c.send({ cmd: 'a' }, 10);
      const aRejected = expect(a).rejects.toThrow(/timeout a/);
      await vi.advanceTimersByTimeAsync(10);
      await aRejected;

      // Request B is sent after A already timed out — it's the new queue head.
      const b = c.send({ cmd: 'b' });

      // The server replies with two lines: A's late (now-orphaned) reply, then B's.
      // Without the `dead` guard, A's line would resolve B's promise instead.
      fakeSocket.emit('data', Buffer.from('{"ok":true,"for":"a"}\n{"ok":true,"for":"b"}\n'));

      await expect(b).resolves.toEqual({ ok: true, for: 'b' });
    } finally {
      vi.useRealTimers();
    }
  });
});
