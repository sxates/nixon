# ADR-0013: Per-meeting folder lease

Status: Accepted
Date: 2026-09-23

## Context

Several jobs touch a meeting's recording folder on disk: the recording saver, batch
retranscription, speaker identification, the retention sweep, the audio compressor
(ADR-0014), meeting delete, interrupted-recording discard, and the transcript save that
writes `meetings.folder_path` back. The recordings mover (specs/0073) adds one more, and it
is the first job that changes *where* a folder is: it renames or copies the folder into the
new recordings folder and rewrites `folder_path`.

Before the mover, nothing coordinated these jobs, and every job trusted the path it had been
handed, often a path the frontend captured minutes earlier. Once folders can move, both
assumptions break: a sweep could delete from a folder that is half copied, and a
retranscription could write into an emptied old folder.

A single global lock would be correct but unusable. A 20 GB cross-volume copy would stall
every summary and every recording start for its whole length.

## Decision

1. **One exclusion primitive, per meeting.** `audio/folder_lease.rs` gives the exclusive
   right to touch one meeting's folder. Every job that reads or writes inside a meeting
   folder, or rewrites `folder_path`, holds that meeting's lease. There is no second lock
   for meeting folders.
2. **The holder is named.** Each lease records a `LeaseHolder` kind: `Recording`,
   `Retranscription`, `Diarization`, `Retention`, `Compression`, `Mover`,
   `FolderPathWrite`, `Delete` or `Discard`. It is used for logs and for the mover's
   "waiting for transcription…" status. A new job that touches folders adds a variant.
3. **Foreground jobs wait; background jobs skip.** Interactive and foreground jobs
   (`acquire`) wait for the current holder: the saver, retranscription, speaker
   identification, delete, discard and the mover. Periodic background jobs (`try_acquire`)
   skip a busy meeting and pick it up on their next tick: the retention sweep and the
   compressor.
4. **Re-read `folder_path` after acquiring.** The folder may have moved while the job
   waited, so a path captured earlier (from the frontend, a list query or a previous tick)
   is only a hint. `reread_folder_path` / `leased_folder_path` return the stored path, and
   fall back to the caller's path only when the row has none.
5. **Only the mover holds more than one lease.** A folder a continued meeting shares
   belongs to several meetings, so the mover takes every one of their leases. It takes them
   in sorted meeting-id order, and no other job ever holds two, so this cannot deadlock. A
   folder whose lease is held by `Recording` is skipped rather than waited for; the next
   gather picks it up.
6. **Lease before engine lock is forbidden; engine lock before lease is allowed.** A
   recording start holds the engine-lifecycle lock while it waits for its meeting's lease.
   A lease holder must therefore release its lease before it takes the engine-lifecycle
   lock (retranscription drops its lease before unloading the engine).
7. **A recording start waits at most 15 seconds.** `RECORDING_LEASE_WAIT` covers a
   same-volume move. When a longer job holds the meeting (a retranscription of it), the
   start fails with a message naming that job instead of a Record button that hangs. The
   engine-lifecycle lock is held for that wait, so the wait is kept short.

## Consequences

- Stale paths are harmless: whatever a job was handed, it works on the folder the database
  names once it holds the lease.
- A move waits only for the one meeting being moved, and other meetings' jobs don't wait
  for the move.
- Coverage is only as good as adoption. A future job that touches a folder without taking
  the lease reintroduces the race; `MeetingsRepository::update_folder_path` carries a
  "lease required" note for that reason.
- The lease is in-process. It doesn't protect against another app, or the user in Finder,
  changing a folder; the mover's verify step and journal cover those cases.

## Alternatives considered

- **A global recordings lock.** Rejected: one long copy would block every other job.
- **Relative `folder_path` under a single root.** Rejected: folders already live under
  several roots (legacy, current, earlier and debug folders), and it would need a migration
  that rewrites every row.
- **Filesystem locks (`flock`) on the folder.** Rejected: a lock file inside the folder
  moves with it, and the jobs that need excluding all run in Nixon's own process.
